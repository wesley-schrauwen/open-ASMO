//! The name of what the cursor rests on in the world, in the bottom bar's message box
//! (Arcanum.exe 0x553a70 hover → 0x57ccf0 → 0x57cd60 / 0x441b60 / 0x553be0).
//!
//! - **When:** as the object under the cursor changes (0x553a70); nothing under it clears the
//!   box. The bars and open windows cover the world.
//! - **The name** (0x441b60): an item's `description` when identified (no magick or tech
//!   complexity, or flagged identified), else its `item_description_unknown`; a portal's,
//!   container's or scenery's `description`; an NPC's `description` once the player's PC has
//!   met it ([`arcanum_sim::LedgerDelta::KnownNpcs`], 0x45f970), else its
//!   `critter_description_unknown`; a PC's name. Numbers below 30000 are `mes/description.mes`
//!   lines (0x4d4210), others `mes/gamedesc.mes`.
//!
//! Not built: the lines under the name (0x553be0: a door's or container's state and hit points,
//! an item's damage, effects, weight and charges, a critter's level and gauges); the examine
//! script (event 0, run by the original before the text: it runs on the host here, which
//! hover does not reach).

use std::collections::HashSet;

use arcanum_formats::mes::MesFile;
use arcanum_formats::object::{Object, Prototypes, TYPE_WALL};
use arcanum_sim::{EntityKind, GroundItemId, NetId};
use bevy::prelude::*;

use crate::actors::Actor;
use crate::main_menu::AppState;
use crate::message_box::{BoxMessage, MessageBox};
use crate::vfs_asset::VfsResource;
use crate::world_render::{ClientMap, MapObject};

/// What a spawned actor is (from its spawn).
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct ActorKind(pub EntityKind);

/// The NPCs the local player's PC has met.
#[derive(Resource, Debug, Default)]
pub struct KnownNpcs(pub HashSet<NetId>);

/// Description numbers from here on are in `mes/gamedesc.mes` (0x4d4210).
pub const GAME_DESCRIPTIONS_FROM: i32 = 30000;

pub struct WorldHoverPlugin;

impl Plugin for WorldHoverPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KnownNpcs>()
            .add_systems(OnExit(AppState::InGame), |mut known: ResMut<KnownNpcs>| known.0.clear())
            .add_systems(Update, hover_world.run_if(in_state(AppState::InGame)));
    }
}

/// The two description files.
#[derive(Default)]
struct Descriptions {
    loaded: bool,
    description: MesFile,
    game: MesFile,
}

impl Descriptions {
    fn text(&self, n: i32) -> Option<String> {
        let file = if n < GAME_DESCRIPTIONS_FROM { &self.description } else { &self.game };
        file.get(u32::try_from(n).ok()?).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
    }
}

/// An item's name number: its description when identified, else its unknown one (0x441b60).
pub fn item_name(object: &Object, prototypes: &Prototypes) -> Option<i32> {
    let complexity = prototypes.int(object, "item_magic_tech_complexity").unwrap_or(0);
    let identified = complexity < 1 || prototypes.int(object, "item_flags").unwrap_or(0) & 1 != 0;
    prototypes.int(object, if identified { "description" } else { "item_description_unknown" })
}

/// A critter's name number: known, its description; else its unknown one (0x45f970).
pub fn critter_name(object: &Object, prototypes: &Prototypes, known: bool) -> Option<i32> {
    prototypes.int(object, if known { "description" } else { "critter_description_unknown" })
}

/// The windows that cover the world.
type Covering<'w> = (Option<Res<'w, crate::character_editor::CharacterPanel>>, Option<Res<'w, crate::inventory_screen::InventoryPanel>>, Option<Res<'w, crate::written_window::WrittenPanel>>, Option<Res<'w, crate::dialogue_ui::DialogueState>>);

type HoverActor<'a> = (Entity, &'a Sprite, &'a bevy::sprite::Anchor, &'a GlobalTransform, &'a Actor, Option<&'a ActorKind>);
type HoverObject<'a> = (Entity, &'a Sprite, &'a bevy::sprite::Anchor, &'a GlobalTransform, &'a MapObject);

/// What is under the cursor, as text for the box.
#[allow(clippy::too_many_arguments)]
fn hover_world(
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    client: Option<Res<ClientMap>>,
    actors: Query<HoverActor>,
    objects: Query<HoverObject>,
    images: Res<Assets<Image>>,
    (party, known, vfs): (Res<crate::actors::Party>, Res<KnownNpcs>, Option<Res<VfsResource>>),
    (panel, inventory, written, dialogue): Covering,
    mut messages: ResMut<MessageBox>,
    time: Option<Res<Time<Real>>>,
    mut texts: Local<Descriptions>,
    mut shown: Local<Option<Entity>>,
) {
    let covered = panel.is_some_and(|p| p.open) || inventory.is_some_and(|p| p.open) || written.is_some_and(|w| w.open.is_some()) || dialogue.is_some_and(|d| d.talking());
    let cursor = windows.iter().find_map(|w| w.cursor_position().map(|c| (c, w.size())));
    let hit = match (cursor, client.as_ref()) {
        (Some((at, size)), Some(_)) if !covered && !crate::hud::covers(size, at, false) => cameras.iter().find_map(|(camera, t)| camera.viewport_to_world_2d(t, at).ok()).and_then(|world| {
            let critters = actors.iter().map(|(e, s, a, t, ..)| (e, s, a, t));
            crate::picking::pick(world, critters, &images).or_else(|| crate::picking::pick(world, objects.iter().filter(|o| o.4.object_type != TYPE_WALL).map(|(e, s, a, t, _)| (e, s, a, t)), &images))
        }),
        _ => None,
    };
    if hit == *shown {
        return;
    }
    let was = *shown;
    *shown = hit;
    let Some(entity) = hit else {
        if was.is_some() {
            messages.clear();
        }
        return;
    };
    if !texts.loaded {
        texts.loaded = true;
        let guard = vfs.as_ref().and_then(|v| v.0.read().ok());
        let read = |p: &str| guard.as_ref().and_then(|v| v.read(p).ok()).and_then(|d| MesFile::parse(&d).ok()).unwrap_or_default();
        texts.description = read("mes/description.mes");
        texts.game = read("mes/gamedesc.mes");
    }
    let Some(client) = client else { return };
    let map = &client.map;
    let name = if let Ok((.., actor, kind)) = actors.get(entity) {
        match kind.map(|k| &k.0) {
            Some(EntityKind::Pc { .. }) => party.status.get(&actor.net_id).map(|s| s.name.clone()),
            Some(&EntityKind::Npc { prototype }) => map.prototypes.get(prototype).and_then(|p| critter_name(p, &map.prototypes, known.0.contains(&actor.net_id))).and_then(|n| texts.text(n)),
            None => None,
        }
    } else if let Ok((.., object)) = objects.get(entity) {
        let item = |o: &Object| if (5..=14).contains(&o.object_type) { item_name(o, &map.prototypes) } else { map.prototypes.int(o, "description") };
        let number = match (object.id, object.ground) {
            (Some(id), _) => map.map_object(id).and_then(item),
            (_, Some(id @ GroundItemId::Container { .. })) => map.container_object(id).and_then(item),
            (_, Some(id)) => map.ground_item(id).and_then(|g| map.prototypes.get(g.prototype).and_then(item)),
            _ => None,
        };
        number.and_then(|n| texts.text(n))
    } else {
        None
    };
    match name {
        Some(name) => {
            messages.show_help(BoxMessage::help(&name), time.map_or(0.0, |t| t.elapsed_secs_f64()));
        }
        None => messages.clear(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcanum_formats::object::{TYPE_NPC, TYPE_WEAPON, Value};
    use arcanum_formats::test_util::ObjectBuilder;

    #[test]
    fn unidentified_items_and_unmet_npcs_show_their_unknown_names() {
        let prototypes = Prototypes::default();
        let sword = |complexity: i32, flags: i32| Object::parse_instance(&ObjectBuilder::new(TYPE_WEAPON).field("description", Value::Int32(100)).field("item_description_unknown", Value::Int32(101)).field("item_magic_tech_complexity", Value::Int32(complexity)).field("item_flags", Value::Int32(flags)).build()).unwrap();
        assert_eq!(item_name(&sword(0, 0), &prototypes), Some(100), "plain: known");
        assert_eq!(item_name(&sword(40, 0), &prototypes), Some(101), "magick, unidentified");
        assert_eq!(item_name(&sword(40, 1), &prototypes), Some(100), "identified");
        let npc = Object::parse_instance(&ObjectBuilder::new(TYPE_NPC).field("description", Value::Int32(200)).field("critter_description_unknown", Value::Int32(201)).build()).unwrap();
        assert_eq!((critter_name(&npc, &prototypes, true), critter_name(&npc, &prototypes, false)), (Some(200), Some(201)));
    }

    #[test]
    fn high_numbers_read_the_game_descriptions() {
        let texts = Descriptions { loaded: true, description: MesFile::parse(b"{100}{Sword}").unwrap(), game: MesFile::parse(b"{30001}{Virgil}").unwrap() };
        assert_eq!((texts.text(100), texts.text(30001), texts.text(5)), (Some("Sword".into()), Some("Virgil".into()), None));
    }
}
