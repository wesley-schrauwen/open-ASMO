//! The in-game inventory screen (Arcanum.exe 0x572240, mode 0: the PC's own inventory; render
//! 0x576520). Positions below are in the 800x400 window between the bars (screen y = y + 41).
//!
//! - **Art:** `PDoll` (art 223) at (0, 0), `Inventor` (221) at (358, 0); `TotalAttack` (676) at
//!   (29, 318), `TotalDefens` (677) at (254, 319), the arrange button (`tiny_butt`, 340) at
//!   (763, 197), the use box (`Use_Box_OFF`, 342) at (703, 290) and the drop box (`Drop_OFF`,
//!   224) at (703, 344).
//! - **Grid:** 10 x 12 cells of 32 pixels from (368, 8); an item at location `row * 10 + col`
//!   is drawn at its natural size, centred in its footprint (`(box - art) / 2`, truncated).
//! - **Paperdoll** (table 0x5cac58): helmet (151, 107, 64, 64), rings (247, 107) and (279, 107)
//!   32x32, amulet (247, 139, 64, 32), weapon (23, 170, 96, 128), shield (247, 171, 96, 128),
//!   armour (119, 171, 128, 160), gauntlets (55, 107, 64, 64), boots (150, 331, 64, 64). An
//!   empty slot shows its cover (0x5caf58: `CVR_*`); a worn item its paperdoll art (weapons
//!   `weapon_paper_doll_aid`, armour `armor_paper_doll_aid`, others the inventory art), centred.
//! - **Text:** "Total weight: %d stone", "Encumbrance: %s" (`mes/critter.mes` 18 + level) and
//!   "Speed: %d" at (145, 29 / 44 / 59, 185, 15) (`mes/inven_ui.mes` 9, 6, 10, 11); gold and the
//!   four ammo counts at (729, 70 + 23 n, 58, 20).
//! - Toggled by the top bar's inventory button and `I`; Escape closes it; opening it closes the
//!   character screen and the other way round. Clicks on it do not walk.
//!
//! Not built: picking items up, moving, using, dropping, the arrange button, total attack and
//! defence, item hover help, barter and loot modes. The original's fonts for the text and
//! numbers were not identified (Flare12Font stands in). The original's encumbrance line also
//! prints a stray number from its stack (`%s: %s (%d)` given two values); it is left out.

use arcanum_formats::art_id::{ArtId, ArtIndex};
use arcanum_formats::object::{Prototypes, TYPE_ARMOR, TYPE_WEAPON};
use arcanum_sim::{AMMO_TYPES, CarriedItem, GOLD, Inventory, Slot, in_grid};
use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use crate::art_loader::ArtFrames;
use crate::create_character::CreationData;
use crate::fonts::{ArcanumFonts, UiFont};
use crate::main_menu::AppState;
use crate::menu_window::{SCREEN_WHITE, TOP_BAR_HEIGHT, at, px, rect};
use crate::resource_pack::paths::interface;

/// The grid's top-left cell, in the band.
pub const GRID: Vec2 = Vec2::new(368.0, 8.0);
/// The paperdoll slots' rects (in the band), by slot.
pub const SLOTS: [(Slot, [f32; 4], &str); 9] = [
    (Slot::Helmet, [151.0, 107.0, 64.0, 64.0], "CVR_Helmet.ART"),
    (Slot::Ring1, [247.0, 107.0, 32.0, 32.0], "CVR_Ring1.ART"),
    (Slot::Ring2, [279.0, 107.0, 32.0, 32.0], "CVR_Ring2.ART"),
    (Slot::Amulet, [247.0, 139.0, 64.0, 32.0], "CVR_Medalion.ART"),
    (Slot::Weapon, [23.0, 170.0, 96.0, 128.0], "CVR_Weapon.ART"),
    (Slot::Shield, [247.0, 171.0, 96.0, 128.0], "CVR_Shield.ART"),
    (Slot::Armor, [119.0, 171.0, 128.0, 160.0], "CVR_Armor.ART"),
    (Slot::Gauntlets, [55.0, 107.0, 64.0, 64.0], "CVR_Gauntlet.ART"),
    (Slot::Boots, [150.0, 331.0, 64.0, 64.0], "CVR_Boot.ART"),
];
/// The weight, encumbrance and speed lines.
const TEXT_LINES: [f32; 3] = [29.0, 44.0, 59.0];
/// Gold, then the ammo counts.
const COUNTS_X: f32 = 729.0;
const COUNTS_Y: [f32; 5] = [70.0, 93.0, 116.0, 139.0, 162.0];

/// Whether the inventory screen is open.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InventoryPanel {
    pub open: bool,
}

/// What the player's own PC carries, as the host last sent it.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct PcInventory(pub Option<Inventory>);

/// The screen's root.
#[derive(Component)]
pub struct InventoryScreen;

/// The items and texts drawn for the inventory (redrawn when it changes).
#[derive(Component)]
pub struct InventoryContent;

/// An item drawn on the screen, for tests.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct DrawnItem(pub CarriedItem);

/// The numbers and lines, for tests.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryText {
    Weight,
    Encumbrance,
    Speed,
    Gold,
    Ammo(usize),
}

pub struct InventoryScreenPlugin;

impl Plugin for InventoryScreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InventoryPanel>()
            .init_resource::<PcInventory>()
            .add_systems(OnExit(AppState::InGame), |mut panel: ResMut<InventoryPanel>, mut inv: ResMut<PcInventory>| {
                *panel = InventoryPanel::default();
                *inv = PcInventory::default();
            })
            .add_systems(PreUpdate, toggle_on_keys.run_if(in_state(AppState::InGame)).after(bevy::input::InputSystems))
            .add_systems(Update, (one_panel_at_a_time, show_panel, show_items).chain().run_if(in_state(AppState::InGame)))
            .add_observer(on_hud_button);
    }
}

fn band(x: f32, y: f32, w: f32, h: f32) -> Rect {
    rect(Vec2::new(x, y + TOP_BAR_HEIGHT), Vec2::new(w, h))
}

fn art(name: &str) -> String {
    format!("{}/{name}", interface::DIR)
}

/// `I` toggles the screen; Escape closes it (before it would leave the game).
fn toggle_on_keys(mut keys: ResMut<ButtonInput<KeyCode>>, mut panel: ResMut<InventoryPanel>) {
    if keys.just_pressed(KeyCode::KeyI) {
        panel.open = !panel.open;
    } else if panel.open && keys.clear_just_pressed(KeyCode::Escape) {
        panel.open = false;
    }
}

fn on_hud_button(activate: On<Activate>, buttons: Query<&crate::hud::HudAction>, mut panel: ResMut<InventoryPanel>) {
    if buttons.get(activate.entity) == Ok(&crate::hud::HudAction::Inventory) {
        panel.open = !panel.open;
    }
}

/// Opening one of the inventory and character screens closes the other.
fn one_panel_at_a_time(mut inventory: ResMut<InventoryPanel>, mut character: ResMut<crate::character_editor::CharacterPanel>) {
    if inventory.is_changed() && inventory.open && character.open {
        character.open = false;
    } else if character.is_changed() && character.open && inventory.open {
        inventory.open = false;
    }
}

fn show_panel(mut commands: Commands, panel: Res<InventoryPanel>, roots: Query<Entity, With<InventoryScreen>>, assets: Res<AssetServer>) {
    if !panel.is_changed() {
        return;
    }
    if !panel.open {
        for root in &roots {
            commands.entity(root).despawn();
        }
        return;
    }
    if !roots.is_empty() {
        return;
    }
    let image = |name: &str, x: f32, y: f32, w: f32, h: f32| (ImageNode::new(assets.load(art(name))), at(band(x, y, w, h)));
    commands
        .spawn((
            InventoryScreen,
            DespawnOnExit(AppState::InGame),
            bevy::picking::Pickable::IGNORE,
            Node { position_type: PositionType::Absolute, top: Val::Px(0.0), left: Val::Percent(50.0), margin: UiRect::left(px(-400.0)), width: px(800.0), height: px(600.0), ..default() },
        ))
        .with_children(|screen| {
            screen.spawn(image("PDoll.ART", 0.0, 0.0, 358.0, 400.0));
            screen.spawn(image("Inventor.ART", 358.0, 0.0, 442.0, 400.0));
            screen.spawn(image("TotalAttack.ART", 29.0, 318.0, 54.0, 71.0));
            screen.spawn(image("TotalDefens.ART", 254.0, 319.0, 54.0, 71.0));
            screen.spawn(image("tiny_butt.ART", 763.0, 197.0, 28.0, 27.0));
            screen.spawn(image("Use_Box_OFF.ART", 703.0, 290.0, 80.0, 43.0));
            screen.spawn(image("Drop_OFF.ART", 703.0, 344.0, 80.0, 43.0));
        });
}

/// Where the item art for `item` is and where it goes: its location's box in the band.
fn placement(item: &CarriedItem, cells: (i32, i32)) -> Option<Rect> {
    if in_grid(item.location) {
        let (col, row) = ((item.location % 10) as f32, (item.location / 10) as f32);
        let at = GRID + Vec2::new(col, row) * 32.0;
        return Some(Rect::from_corners(at, at + Vec2::new(cells.0 as f32, cells.1 as f32) * 32.0));
    }
    SLOTS.iter().find(|(s, ..)| *s as i32 == item.location).map(|(_, r, _)| Rect::from_corners(Vec2::new(r[0], r[1]), Vec2::new(r[0] + r[2], r[1] + r[3])))
}

/// The top-left corner of art `size` centred in `inside`, as the original rounds it: grid
/// items `(box - art) / 2` truncated towards zero, slots `(box - art) >> 1`.
pub fn centred(inside: Rect, size: UVec2, grid: bool) -> Vec2 {
    let gap = |b: f32, a: u32| {
        let d = b as i32 - a as i32;
        if grid { d / 2 } else { d >> 1 }
    };
    inside.min + Vec2::new(gap(inside.width(), size.x) as f32, gap(inside.height(), size.y) as f32)
}

/// The art an item shows at its location: worn weapons and armour their paperdoll art.
fn item_art(prototypes: &Prototypes, item: &CarriedItem) -> Option<ArtId> {
    let p = prototypes.get(item.prototype)?;
    let field = match p.object_type {
        TYPE_WEAPON if !in_grid(item.location) => "weapon_paper_doll_aid",
        TYPE_ARMOR if !in_grid(item.location) => "armor_paper_doll_aid",
        _ => "item_inv_aid",
    };
    p.int(field).or_else(|| p.int("item_inv_aid")).map(|v| ArtId(v as u32))
}

/// Item art handles by path.
type ArtHandles = std::collections::HashMap<String, Handle<ArtFrames>>;

#[allow(clippy::too_many_arguments)]
fn show_items(
    mut commands: Commands,
    inventory: Res<PcInventory>,
    sheet: Res<crate::pc_sheet::SheetHost>,
    client: Option<Res<crate::world_render::ClientMap>>,
    data: Option<Res<CreationData>>,
    fonts: Option<Res<ArcanumFonts>>,
    roots: Query<Entity, With<InventoryScreen>>,
    contents: Query<Entity, With<InventoryContent>>,
    added: Query<(), Added<InventoryScreen>>,
    (assets, frames, mut pending, mut handles): (Res<AssetServer>, Res<Assets<ArtFrames>>, Local<bool>, Local<ArtHandles>),
) {
    let Ok(root) = roots.single() else { return };
    if !inventory.is_changed() && added.is_empty() && !*pending {
        return;
    }
    *pending = false;
    for old in &contents {
        commands.entity(old).despawn();
    }
    let Some(inv) = &inventory.0 else { return };
    let empty = Prototypes::default();
    let prototypes = client.as_ref().map_or(&empty, |c| &c.map.prototypes);
    let index = client.as_ref().and_then(|c| c.art_index());
    let fonts = fonts.map(|f| f.clone()).unwrap_or_default();
    let mut content = Vec::new();
    for (slot, r, cover) in SLOTS {
        if inv.worn(slot).is_none() {
            content.push(commands.spawn((InventoryContent, bevy::picking::Pickable::IGNORE, ImageNode::new(assets.load(art(cover))), Node { width: Val::Auto, height: Val::Auto, ..at(band(r[0], r[1], r[2], r[3])) })).id());
        }
    }
    for item in &inv.items {
        let Some(path) = item_art(prototypes, item).and_then(|id| index.and_then(|i: &ArtIndex| i.path(id))) else { continue };
        // Kept: the art would unload before it loads once the last handle is dropped.
        let handle = handles.entry(path.clone()).or_insert_with(|| assets.load(path)).clone();
        let Some((image, _, size)) = frames.get(&handle).and_then(|f| f.rotation(0)?.frame(0).map(|(i, c, s)| (i.clone(), c, s))) else {
            // Not loaded yet: draw again next frame.
            *pending = true;
            continue;
        };
        let cells = arcanum_sim::cells_for(size.x, size.y);
        let Some(inside) = placement(item, cells) else { continue };
        let corner = centred(inside, size, in_grid(item.location));
        content.push(commands.spawn((InventoryContent, DrawnItem(item.clone()), bevy::picking::Pickable::IGNORE, ImageNode::new(image), at(band(corner.x, corner.y, size.x as f32, size.y as f32)))).id());
    }
    // The texts.
    let rules = data.as_ref().map(|d| d.rules.clone()).unwrap_or_default();
    let c = sheet.sheet().cloned().unwrap_or_default();
    // Weight needs no art sizes.
    let info = arcanum_sim::item_info(prototypes, |_| None);
    let aptitude = rules.aptitude(&c);
    let carry = rules.derived(&c).carry_weight;
    let total = inv.total_weight(&info, aptitude);
    let level = arcanum_sim::encumbrance(total, carry);
    let label = |id: u32, fallback: &str| data.as_ref().and_then(|d| d.inven_ui.get(id)).unwrap_or(fallback).to_string();
    let level_name = data.as_ref().and_then(|d| d.critter.get(18 + level as u32)).unwrap_or("").to_string();
    let lines = [
        (InventoryText::Weight, format!("{}: {} {}", label(9, "Total weight"), total, label(6, "stone"))),
        (InventoryText::Encumbrance, format!("{}: {}", label(10, "Encumbrance"), level_name)),
        (InventoryText::Speed, format!("{}: {}", label(11, "Speed"), rules.derived(&c).speed)),
    ];
    for ((which, text), y) in lines.into_iter().zip(TEXT_LINES) {
        content.push(text_cell(&mut commands, &fonts, which, &text, band(145.0, y, 185.0, 15.0)));
    }
    let count = |proto: Option<u32>, kind: Option<i32>| -> i32 {
        inv.items.iter().filter(|i| proto.is_none_or(|p| i.prototype == p) && kind.is_none_or(|k| prototypes.get(i.prototype).and_then(|p| p.int("ammo_type")) == Some(k) && prototypes.get(i.prototype).is_some_and(|p| p.object_type == arcanum_formats::object::TYPE_AMMO))).map(|i| i.quantity).sum()
    };
    content.push(text_cell(&mut commands, &fonts, InventoryText::Gold, &count(Some(GOLD), None).to_string(), band(COUNTS_X, COUNTS_Y[0], 58.0, 20.0)));
    for a in 0..AMMO_TYPES {
        content.push(text_cell(&mut commands, &fonts, InventoryText::Ammo(a), &count(None, Some(a as i32)).to_string(), band(COUNTS_X, COUNTS_Y[a + 1], 58.0, 20.0)));
    }
    commands.entity(root).add_children(&content);
}

fn text_cell(commands: &mut Commands, fonts: &ArcanumFonts, which: InventoryText, text: &str, r: Rect) -> Entity {
    commands
        .spawn((InventoryContent, bevy::picking::Pickable::IGNORE, Node { overflow: Overflow::clip(), ..at(r) }))
        .with_children(|cell| {
            let mut t = cell.spawn((which, TextColor(SCREEN_WHITE)));
            fonts.insert_text(&mut t, text, UiFont::Body);
        })
        .id()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_sit_centred_in_their_footprint() {
        let item = CarriedItem { prototype: 1, location: 12, quantity: 0 };
        let inside = placement(&item, (2, 3)).unwrap();
        assert_eq!(inside, Rect::from_corners(Vec2::new(432.0, 40.0), Vec2::new(496.0, 136.0)));
        assert_eq!(centred(inside, UVec2::new(41, 90), true), Vec2::new(443.0, 43.0));
        let sword = CarriedItem { prototype: 1, location: Slot::Weapon as i32, quantity: 0 };
        assert_eq!(placement(&sword, (1, 1)).unwrap().min, Vec2::new(23.0, 170.0));
        assert_eq!(centred(placement(&sword, (1, 1)).unwrap(), UVec2::new(101, 128), false), Vec2::new(20.0, 170.0), "an arithmetic shift rounds down");
    }
}
