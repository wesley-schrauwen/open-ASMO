//! The fight as the player sees it (`plans/combat.md` step 3; [`arcanum_sim::combat`]).
//!
//! - **Combat mode:** the bottom bar's combat button (art 470 at (86, 457), 0x54ebf0) toggles it.
//!   In combat mode, or while the player's PC is in a fight, a click on an NPC attacks it
//!   (`PlayerIntent::Attack`; 0x54ed30 → 0x4445a0): the host walks a melee attacker next to it.
//! - **The AP bar** (0x56ed20 / 0x56f430): while the player's PC is in a fight, art 294 at
//!   (222, 412) above the bottom bar, its End Turn button (art 297) at (320, 9) in it, and on the
//!   PC's turn one green light (art 295) a point left, at most 20, at x 22, 35, … (table
//!   0x5caa48), y 10. End Turn sends `PlayerIntent::EndTurn`.
//! - **Floats** (`mes/combat.mes`, 0x4b4390 / 0x4b3600): over the target, "HT n" for hit points
//!   and "FT n" for fatigue, "Dodge!", "Critical hit", "Damage +50% / +100% / +200%",
//!   "Critical miss" over the attacker, "Blinded", "Crippled arm", "Crippled leg", "Armor
//!   damaged" / "broken", "Weapon damaged" / "broken"; white over NPCs, red over PCs.
//!
//! - **Animations:** the attacker faces its target and plays its attack once ([`attack_anim`],
//!   0x42b9c0: 20 or 21 by body); an NPC an attack killed plays its death from the first frame
//!   when the host spawns it again dead ([`Dying`]; the host picks the death, 0x4b5306); one
//!   knocked down falls (7) and gets up (8); one knocked out falls when spawned again lying and
//!   gets up (8) when spawned again standing ([`Lying`]).
//!
//! - **Planning** ([`plan_move`], 0x56f430): on the PC's turn, the tile or NPC under the cursor
//!   gives the planned cost (2 a step; an NPC: the steps into reach and the attack): the bar
//!   lights orange (art 296) for the points it would spend, red (315) for those it lacks.
//! - **Cursor** ([`crate::cursor`]): in combat mode or a fight the battle cursor (art 353), over
//!   an NPC `hoverattack` (555).
//!
//! Not built: the hit effects (AnimFX by damage, 0x433020), projectiles (0x4b39b0),
//! called shots, the button's pressed look.

use arcanum_sim::{FloatColour, LedgerDelta, NetId, PlayerIntent};
use bevy::prelude::*;

use crate::main_menu::AppState;
use crate::menu_window::{at, px};
use crate::message_box::InterfaceArt;
use crate::sim_bridge::{HostLink, LocalPc};
use crate::vfs_asset::VfsResource;

/// The AP bar's top-left on the 800x600 screen, its art, the End Turn button's and the lights'.
pub const AP_BAR_AT: Vec2 = Vec2::new(222.0, 412.0);
pub const AP_BAR_ART: u32 = 294;
pub const END_TURN_ART: u32 = 297;
pub const END_TURN_AT: Vec2 = Vec2::new(320.0, 9.0);
pub const AP_LIGHT_ART: u32 = 295;
/// The lights of points a planned move spends, and of points it lacks.
pub const AP_PLANNED_ART: u32 = 296;
pub const AP_SHORT_ART: u32 = 315;
/// The lights' x in the bar (table 0x5caa48), y 10.
pub const AP_LIGHT_X: [f32; 20] = [22.0, 35.0, 48.0, 61.0, 74.0, 93.0, 106.0, 119.0, 132.0, 145.0, 164.0, 177.0, 190.0, 203.0, 216.0, 235.0, 248.0, 261.0, 274.0, 287.0];
pub const AP_LIGHT_Y: f32 = 10.0;

/// The fight as the host last told it, and the player's combat mode.
#[derive(Resource, Debug, Default)]
pub struct CombatView {
    pub order: Vec<NetId>,
    pub turn: Option<NetId>,
    pub ap: i32,
    /// The combat button is on.
    pub mode: bool,
    /// What the move under the cursor would cost, on the PC's turn ([`plan_move`]).
    pub planned: i32,
    /// The cursor is on an NPC.
    pub over_npc: bool,
}

impl CombatView {
    /// Whether `pc` is in the fight.
    pub fn fighting(&self, pc: Option<NetId>) -> bool {
        pc.is_some_and(|pc| self.order.contains(&pc))
    }

    /// Whether a click on an NPC attacks it.
    pub fn attacks(&self, pc: Option<NetId>) -> bool {
        self.mode || self.fighting(pc)
    }
}

/// The floats an attack gives (`combat.mes` lines): over whom, the text, red over PCs.
pub fn attack_floats(attacker: NetId, target: NetId, outcome: &arcanum_sim::combat::AttackOutcome, target_is_pc: bool, mes: &dyn Fn(u32) -> String) -> Vec<(NetId, String, FloatColour)> {
    let colour = if target_is_pc { FloatColour::Red } else { FloatColour::White };
    let mut out = Vec::new();
    if outcome.fumbled {
        out.push((attacker, mes(2), FloatColour::White));
        // What befell the attacker (0x4b4390): "Weapon destroyed" (22), "Ammo lost" (9), its own
        // stun, fall and injuries.
        use arcanum_sim::combat::{AMMO_LOST, AMMO_LOST_ALL, WEAPON_DESTROYED};
        for (bit, line) in [(WEAPON_DESTROYED, 22), (AMMO_LOST_ALL | AMMO_LOST, 9)] {
            if outcome.effects & bit != 0 {
                out.push((attacker, mes(line), FloatColour::White));
            }
        }
        out.extend(effect_floats(outcome.fumble_effects, outcome.fumble_injured, mes).into_iter().map(|t| (attacker, t, FloatColour::White)));
    }
    if outcome.dodged {
        out.push((target, mes(11), FloatColour::White));
    }
    if outcome.hit {
        if outcome.critical {
            out.push((target, mes(12), colour));
            let line = match outcome.multiplier {
                300 => 15,
                200 => 14,
                _ => 13,
            };
            out.push((target, mes(line), colour));
            out.extend(effect_floats(outcome.effects, outcome.injured, mes).into_iter().map(|t| (target, t, colour)));
            // Items (0x4b4390): "Armor damaged" (7), "Weapon damaged" (8), "Weapon broken" (23),
            // "Armor broken" (24).
            use arcanum_sim::combat::{ARMOUR_BROKEN, ARMOUR_DAMAGED, WEAPON_BROKEN, WEAPON_DAMAGED};
            for (bit, line) in [(ARMOUR_DAMAGED, 7), (WEAPON_DAMAGED, 8), (ARMOUR_BROKEN, 24), (WEAPON_BROKEN, 23)] {
                if outcome.items & bit != 0 {
                    out.push((target, mes(line), colour));
                }
            }
        }
        if outcome.hp > 0 {
            out.push((target, format!("{} {}", mes(1), outcome.hp), colour));
        }
        if outcome.fatigue > 0 {
            out.push((target, format!("{} {}", mes(10), outcome.fatigue), FloatColour::Red));
        }
    }
    out
}

/// NPCs' hit points and fatigue as the host last told them ([`LedgerDelta::NpcHealth`]); the
/// others are whole.
#[derive(Resource, Debug, Default)]
pub struct NpcHealth(pub std::collections::HashMap<NetId, (arcanum_sim::Pool, arcanum_sim::Pool)>);

/// NPCs an attack killed: when the host spawns them again dead, they play their death
/// ([`crate::actors::Actor::action`]) instead of appearing on its last frame.
#[derive(Resource, Debug, Default)]
pub struct Dying(pub std::collections::HashSet<NetId>);

/// NPCs last spawned lying (knocked out or dead: [`LYING_ANIMS`]): spawned again standing, they
/// play the get-up ([`crate::actors::ANIM_GET_UP`]; builder's call for one raised from the dead).
#[derive(Resource, Debug, Default)]
pub struct Lying(pub std::collections::HashSet<NetId>);

/// The animations a critter lies in: knocked down 7 (also the light death), the deaths 17-19
/// (0x4b5306).
pub const LYING_ANIMS: [u32; 4] = [crate::actors::ANIM_KNOCKED_DOWN, 17, 18, 19];

impl Lying {
    /// Notes NPC `net_id` spawned with `art`: true when it lay before and stands now.
    pub fn gets_up(&mut self, net_id: NetId, art: arcanum_formats::art_id::ArtId) -> bool {
        if LYING_ANIMS.contains(&((art.0 >> 6) & 0x1f)) {
            self.0.insert(net_id);
            false
        } else {
            self.0.remove(&net_id)
        }
    }
}

/// The attack animation of critter art `art` (0x42b9c0 → 0x503f60): 20 when its body's (table
/// 0x5be980: human, elf 1, half-ogre 2) or monster's (0x5be994, by bits 23-27) entry is set, else
/// 21; unique NPCs 20. Not built: 14 for thrown weapons, 21 forced by `flags` 0x80 or while
/// knocked down.
pub fn attack_anim(art: arcanum_formats::art_id::ArtId) -> u32 {
    const BODIES: [u32; 5] = [1, 0, 0, 2, 1];
    const MONSTERS: [u32; 32] = [0, 0, 1, 2, 1, 1, 1, 1, 2, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 1, 1, 0, 2, 2, 2, 1, 1, 1];
    let set = match art.art_type() {
        2 => BODIES.get(((art.0 >> 24) & 7) as usize).copied().unwrap_or(1),
        0xc => MONSTERS[((art.0 >> 23) & 0x1f) as usize],
        _ => 1,
    };
    if set != 0 { ANIM_ATTACK } else { ANIM_ATTACK_ALT }
}

/// The two attack animations (0x42b9c0).
pub const ANIM_ATTACK: u32 = 20;
pub const ANIM_ATTACK_ALT: u32 = 21;

/// The rotation from `from` facing `to` (one of the eight).
fn facing(from: arcanum_formats::map::TileLoc, to: arcanum_formats::map::TileLoc) -> Option<usize> {
    let step = arcanum_formats::map::TileLoc { x: from.x.checked_add_signed((i64::from(to.x) - i64::from(from.x)).signum() as i32)?, y: from.y.checked_add_signed((i64::from(to.y) - i64::from(from.y)).signum() as i32)? };
    arcanum_sim::rotation_of_step(from, step)
}

/// A critical's effect lines (0x4b4390): "Stunned" (16), "Unconscious" (17), "Knocked down" (18), "Blinded" (4),
/// "Crippled arm" (5), "Crippled leg" (6).
fn effect_floats(effects: u8, injured: i32, mes: &dyn Fn(u32) -> String) -> Vec<String> {
    use arcanum_sim::combat::{KNOCKED_DOWN, KNOCKED_OUT, OCF_BLINDED, OCF_CRIPPLED_ARM, OCF_CRIPPLED_ARMS, OCF_CRIPPLED_LEGS, STUNNED};
    let mut out = Vec::new();
    for (bit, line) in [(STUNNED, 16), (KNOCKED_OUT, 17), (KNOCKED_DOWN, 18)] {
        if effects & bit != 0 {
            out.push(mes(line));
        }
    }
    for (bits, line) in [(OCF_BLINDED, 4), (OCF_CRIPPLED_ARM | OCF_CRIPPLED_ARMS, 5), (OCF_CRIPPLED_LEGS, 6)] {
        if injured & bits != 0 {
            out.push(mes(line));
        }
    }
    out
}

/// The fight's deltas from the bridge, for [`apply_combat_deltas`].
#[derive(Resource, Debug, Default)]
pub struct CombatDeltas(pub Vec<LedgerDelta>);

/// Applies the fight's deltas (`mes/combat.mes` read on first use).
#[allow(clippy::too_many_arguments)]
fn apply_combat_deltas(
    mut deltas: ResMut<CombatDeltas>,
    mut view: ResMut<CombatView>,
    mut floats: Option<ResMut<crate::dialogue_ui::Floats>>,
    party: Res<crate::actors::Party>,
    vfs: Option<Res<VfsResource>>,
    mut mes: Local<Option<arcanum_formats::mes::MesFile>>,
    mut actors: Query<&mut crate::actors::Actor>,
    mut dying: ResMut<Dying>,
    mut health: ResMut<NpcHealth>,
) {
    if deltas.0.is_empty() {
        return;
    }
    if mes.is_none() {
        let guard = vfs.as_ref().and_then(|v| v.0.read().ok());
        *mes = Some(guard.and_then(|v| v.read(COMBAT_MES).ok()).and_then(|d| arcanum_formats::mes::MesFile::parse(&d).ok()).unwrap_or_default());
    }
    let text = |k: u32| mes.as_ref().and_then(|m| m.get(k)).map(|t| t.trim().to_string()).unwrap_or_default();
    let is_pc = |id: NetId| party.status.contains_key(&id);
    for delta in std::mem::take(&mut deltas.0) {
        if let LedgerDelta::NpcHealth { net_id, hit_points, fatigue } = delta {
            health.0.insert(net_id, (hit_points, fatigue));
            continue;
        }
        apply(&delta, &mut view, floats.as_deref_mut(), &is_pc, &text);
        // The attacker swings, facing its target; a killed NPC's death plays when it comes back.
        if let LedgerDelta::Attacked { attacker, target, outcome } = &delta {
            let to = actors.iter().find(|a| a.net_id == *target).map(|a| (a.at, a.art));
            if let Some(mut a) = actors.iter_mut().find(|a| a.net_id == *attacker) {
                a.action = Some((attack_anim(a.art), 0.0));
                if let Some(r) = to.and_then(|(at, _)| facing(a.at, at)) {
                    a.rotation = r;
                }
            }
            // Killed or knocked out: the fall plays when the host spawns it again lying.
            use arcanum_sim::combat::{KNOCKED_DOWN, KNOCKED_OUT};
            let out = outcome.effects & KNOCKED_OUT != 0;
            if outcome.killed || out {
                dying.0.insert(*target);
            }
            // Knocked down (goal 62, animation 7), then up again (goal 63, animation 8).
            for (who, fell) in [(*target, outcome.effects & KNOCKED_DOWN != 0 && !outcome.killed && !out), (*attacker, outcome.fumble_effects & KNOCKED_DOWN != 0)] {
                if fell && let Some(mut a) = actors.iter_mut().find(|a| a.net_id == who) {
                    a.action = Some((crate::actors::ANIM_KNOCKED_DOWN, 0.0));
                    a.then = Some(crate::actors::ANIM_GET_UP);
                }
            }
        }
    }
}

/// `mes/combat.mes`.
pub const COMBAT_MES: &str = "mes/combat.mes";

/// The AP bar's root.
#[derive(Component)]
struct ApBar;

/// The End Turn button.
#[derive(Component)]
struct EndTurnButton;

pub struct CombatUiPlugin;

impl Plugin for CombatUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CombatView>()
            .init_resource::<Dying>()
            .init_resource::<Lying>()
            .init_resource::<NpcHealth>()
            .init_resource::<CombatDeltas>()
            .add_systems(OnExit(AppState::InGame), |mut v: ResMut<CombatView>| *v = CombatView::default())
            .add_observer(on_combat_button)
            .add_systems(Update, (apply_combat_deltas, plan_move, show_ap_bar, end_turn).chain().run_if(in_state(AppState::InGame)));
    }
}

/// The combat button toggles combat mode.
fn on_combat_button(activate: On<bevy::ui_widgets::Activate>, buttons: Query<&crate::hud::HudAction>, mut view: ResMut<CombatView>) {
    if let Ok(crate::hud::HudAction::Combat) = buttons.get(activate.entity) {
        view.mode = !view.mode;
    }
}

/// Builds the AP bar again when the fight changes.
#[allow(clippy::too_many_arguments)]
fn show_ap_bar(mut commands: Commands, view: Res<CombatView>, local: Res<LocalPc>, roots: Query<Entity, With<ApBar>>, assets: Res<AssetServer>, vfs: Option<Res<VfsResource>>, mut art: Local<InterfaceArt>, mut shown: Local<Option<(bool, i32, i32)>>) {
    let pc = local.0.map(|(id, _)| id);
    let want = view.fighting(pc).then(|| (view.turn == pc, view.ap, view.planned));
    if *shown == want && !(want.is_some() && roots.is_empty()) {
        return;
    }
    *shown = want;
    for root in &roots {
        commands.entity(root).despawn();
    }
    let Some((mine, ap, planned)) = want else { return };
    let vfs = vfs.as_deref();
    let (Some(bar), Some(size)) = (art.image(AP_BAR_ART, &assets, vfs), art.size(AP_BAR_ART, vfs)) else { return };
    let end = art.image(END_TURN_ART, &assets, vfs);
    let end_size = art.size(END_TURN_ART, vfs).unwrap_or(Vec2::new(40.0, 20.0));
    let lights: Vec<(Option<Handle<Image>>, Vec2)> = light_arts(ap, planned).into_iter().map(|n| (art.image(n, &assets, vfs), art.size(n, vfs).unwrap_or(Vec2::splat(10.0)))).collect();
    commands
        .spawn((
            ApBar,
            DespawnOnExit(AppState::InGame),
            GlobalZIndex(4),
            Node { position_type: PositionType::Absolute, top: Val::Px(0.0), left: Val::Percent(50.0), margin: UiRect::left(px(-400.0)), width: px(800.0), height: px(600.0), ..default() },
            bevy::picking::Pickable::IGNORE,
        ))
        .with_children(|screen| {
            let place = |corner: Vec2, size: Vec2| at(Rect::from_corners(corner, corner + size));
            screen.spawn((bevy::picking::Pickable::IGNORE, ImageNode::new(bar), place(AP_BAR_AT, size)));
            if let Some(end) = end {
                screen.spawn((EndTurnButton, bevy::ui::RelativeCursorPosition::default(), ImageNode::new(end), place(AP_BAR_AT + END_TURN_AT, end_size)));
            }
            if mine {
                for (x, (light, size)) in AP_LIGHT_X.iter().zip(&lights) {
                    if let Some(light) = light {
                        screen.spawn((bevy::picking::Pickable::IGNORE, ImageNode::new(light.clone()), place(AP_BAR_AT + Vec2::new(*x, AP_LIGHT_Y), *size)));
                    }
                }
            }
        });
}

/// The lights' art, left to right, for `ap` points left and a planned move of `planned`: green
/// for what stays, orange for what it spends, red for what it lacks; at most 20.
pub fn light_arts(ap: i32, planned: i32) -> Vec<u32> {
    let (ap, planned) = (ap.max(0), planned.max(0));
    let green = (ap - planned).max(0);
    let orange = ap.min(planned);
    let red = (planned - ap).max(0);
    std::iter::repeat_n(AP_LIGHT_ART, green as usize).chain(std::iter::repeat_n(AP_PLANNED_ART, orange as usize)).chain(std::iter::repeat_n(AP_SHORT_ART, red as usize)).take(20).collect()
}

/// What a plan was made for: the NPC, the tile under the cursor, the PC's tile.
type PlanKey = (Option<NetId>, Option<arcanum_formats::map::TileLoc>, arcanum_formats::map::TileLoc);

type PlanActors<'a> = (Entity, &'a Sprite, &'a bevy::sprite::Anchor, &'a GlobalTransform, &'a crate::actors::Actor);

/// What the move under the cursor would cost on the PC's turn (0x56f430): an NPC, the steps into
/// its reach and the attack's cost; a tile, 2 a step of the path there; and whether the cursor is
/// on an NPC (the cursor's look).
#[allow(clippy::too_many_arguments)]
fn plan_move(
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    client: Option<Res<crate::world_render::ClientMap>>,
    actors: Query<PlanActors>,
    images: Res<Assets<Image>>,
    local: Res<LocalPc>,
    (sheet_host, pc_inventory): (Res<crate::pc_sheet::SheetHost>, Res<crate::inventory_screen::PcInventory>),
    mut view: ResMut<CombatView>,
    mut last: Local<Option<PlanKey>>,
) {
    let pc = local.0.map(|(id, _)| id);
    let (Some(client), Some(origin)) = (client.as_ref(), client.as_ref().and_then(|c| c.origin)) else { return };
    let world = windows.iter().find_map(|w| w.cursor_position()).and_then(|c| cameras.iter().find_map(|(camera, t)| camera.viewport_to_world_2d(t, c).ok()));
    let npc = world.and_then(|w| crate::picking::pick(w, actors.iter().filter(|a| !a.4.pc).map(|(e, s, a, t, _)| (e, s, a, t)), &images)).and_then(|e| actors.get(e).ok()).map(|a| (a.4.net_id, a.4.at));
    let over_npc = npc.is_some();
    if view.over_npc != over_npc {
        view.over_npc = over_npc;
    }
    let me = actors.iter().find(|a| Some(a.4.net_id) == pc).map(|a| a.4.at);
    let (Some(me), true) = (me, view.turn.is_some() && view.turn == pc) else {
        if view.planned != 0 {
            view.planned = 0;
        }
        return;
    };
    let tile = world.and_then(|w| crate::isometric::world_to_tile(w, origin));
    let key = (npc.map(|n| n.0), tile, me);
    if *last == Some(key) {
        return;
    }
    *last = Some(key);
    let map = &client.map;
    let planned = match npc {
        Some((_, at)) => {
            let weapon = pc_inventory.shown().and_then(|inv| inv.items.iter().find(|i| i.location == arcanum_sim::combat::WEAPON_SLOT)).and_then(|i| map.prototypes.get(i.prototype)).filter(|o| o.object_type == arcanum_formats::object::TYPE_WEAPON);
            let trained = sheet_host.sheet().map(|s| [arcanum_sim::combat::SKILL_BOW, arcanum_sim::combat::SKILL_MELEE, arcanum_sim::combat::SKILL_THROWING, arcanum_sim::combat::SKILL_FIREARMS].into_iter().filter(|&k| s.training[k] > 0).collect()).unwrap_or_default();
            let fighter = arcanum_sim::combat::Fighter { weapon: weapon.map(|w| arcanum_sim::combat::weapon_of(w, &map.prototypes)), trained, ..Default::default() };
            let steps = if arcanum_sim::combat::reach_between(me, at) <= fighter.reach() {
                0
            } else {
                arcanum_sim::combat::neighbours(at).into_iter().filter_map(|g| arcanum_sim::find_path(map, me, g).ok()).map(|p| p.iter().position(|&t| arcanum_sim::combat::reach_between(t, at) <= fighter.reach()).map_or(p.len(), |i| i + 1)).min().unwrap_or(0)
            };
            steps as i32 * arcanum_sim::combat::STEP_AP + fighter.attack_cost()
        }
        None => tile.filter(|&t| t != me).and_then(|t| arcanum_sim::find_path(map, me, t).ok()).map_or(0, |p| p.len() as i32 * arcanum_sim::combat::STEP_AP),
    };
    if view.planned != planned {
        view.planned = planned;
    }
}

/// A click on End Turn ends the PC's turn.
fn end_turn(buttons: Res<ButtonInput<MouseButton>>, end: Query<&bevy::ui::RelativeCursorPosition, With<EndTurnButton>>, link: Option<Res<HostLink>>) {
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    if end.iter().any(|c| c.cursor_over()) && let Some(link) = link {
        let _ = link.0.send_intent(PlayerIntent::EndTurn);
    }
}

/// Whether the AP bar is at `point` (800x600 screen pixels) while shown: it takes the click.
pub fn covers(view: &CombatView, pc: Option<NetId>, point: Vec2) -> bool {
    view.fighting(pc) && Rect::from_corners(AP_BAR_AT, AP_BAR_AT + Vec2::new(340.0, 29.0)).contains(point)
}

/// Applies the fight's deltas: the view, and floats over the critters.
pub fn apply(delta: &LedgerDelta, view: &mut CombatView, floats: Option<&mut crate::dialogue_ui::Floats>, pcs: &dyn Fn(NetId) -> bool, mes: &dyn Fn(u32) -> String) {
    match delta {
        LedgerDelta::Combat { order, turn, ap } => {
            view.order = order.clone();
            view.turn = *turn;
            view.ap = *ap;
        }
        LedgerDelta::Attacked { attacker, target, outcome } => {
            if let Some(floats) = floats {
                for (over, text, colour) in attack_floats(*attacker, *target, outcome, pcs(*target), mes) {
                    floats.0.push((over, text, colour, None));
                }
            }
        }
        // Poison's hurt (0x4b1170 → 0x4b4390): "HT n", red over PCs.
        LedgerDelta::Hurt { net_id, hit_points } => {
            if let Some(floats) = floats {
                let colour = if pcs(*net_id) { FloatColour::Red } else { FloatColour::White };
                floats.0.push((*net_id, format!("{} {}", mes(1), hit_points), colour, None));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_npc_that_lay_gets_up_when_spawned_standing() {
        use arcanum_formats::art_id::CritterArt;
        let art = |anim| CritterArt { anim, male: true, ..Default::default() }.id();
        let mut lying = Lying::default();
        let id = NetId(7);
        assert!(!lying.gets_up(id, art(0)), "standing from the start");
        assert!(!lying.gets_up(id, art(crate::actors::ANIM_KNOCKED_DOWN)), "knocked out");
        assert!(lying.gets_up(id, art(0)), "awake");
        assert!(!lying.gets_up(id, art(0)), "once");
        assert!(!lying.gets_up(id, art(18)) && lying.gets_up(id, art(0)), "raised from the dead");
    }

    #[test]
    fn attackers_swing_by_their_body_facing_the_target() {
        use arcanum_formats::art_id::CritterArt;
        let body = |body| CritterArt { body, male: true, ..Default::default() }.id();
        assert_eq!([0, 1, 2, 3, 4].map(|b| attack_anim(body(b))), [20, 21, 21, 20, 20], "table 0x5be980");
        let t = |x, y| arcanum_formats::map::TileLoc { x, y };
        assert_eq!(facing(t(10, 10), t(15, 10)), arcanum_sim::rotation_of_step(t(10, 10), t(11, 10)));
        assert_eq!(facing(t(10, 10), t(10, 10)), None);
    }

    #[test]
    fn attacks_float_their_combat_lines() {
        let mes = |k: u32| match k {
            1 => "HT".to_string(),
            10 => "FT".into(),
            11 => "Dodge!".into(),
            12 => "Critical hit".into(),
            14 => "Damage +100%".into(),
            6 => "Crippled leg".into(),
            24 => "Armor broken".into(),
            16 => "Stunned".into(),
            _ => "?".into(),
        };
        let hit = arcanum_sim::combat::AttackOutcome { hit: true, critical: true, multiplier: 200, hp: 8, fatigue: 3, injured: arcanum_sim::combat::OCF_CRIPPLED_LEGS, items: arcanum_sim::combat::ARMOUR_BROKEN, ..Default::default() };
        let floats = attack_floats(NetId(1), NetId(2), &hit, true, &mes);
        let texts: Vec<&str> = floats.iter().map(|f| f.1.as_str()).collect();
        assert_eq!(texts, ["Critical hit", "Damage +100%", "Crippled leg", "Armor broken", "HT 8", "FT 3"]);
        assert!(floats.iter().all(|f| f.0 == NetId(2) && f.2 == FloatColour::Red), "over the PC, red");
        let fumble = arcanum_sim::combat::AttackOutcome { fumbled: true, effects: arcanum_sim::combat::WEAPON_DESTROYED, fumble_effects: arcanum_sim::combat::STUNNED, ..Default::default() };
        let texts: Vec<(NetId, String)> = attack_floats(NetId(1), NetId(2), &fumble, false, &mes).into_iter().map(|f| (f.0, f.1)).collect();
        assert_eq!(texts, [(NetId(1), "?".to_string()), (NetId(1), "?".into()), (NetId(1), "Stunned".into())], "all over the attacker");
        let dodged = arcanum_sim::combat::AttackOutcome { dodged: true, ..Default::default() };
        assert_eq!(attack_floats(NetId(1), NetId(2), &dodged, false, &mes), [(NetId(2), "Dodge!".into(), FloatColour::White)]);
        let view = CombatView { order: vec![NetId(1), NetId(2)], turn: Some(NetId(1)), ap: 7, mode: false, ..Default::default() };
        assert_eq!(light_arts(7, 0), [AP_LIGHT_ART; 7].to_vec());
        assert_eq!(light_arts(5, 2), [AP_LIGHT_ART, AP_LIGHT_ART, AP_LIGHT_ART, AP_PLANNED_ART, AP_PLANNED_ART].to_vec());
        assert_eq!(light_arts(3, 5), [AP_PLANNED_ART, AP_PLANNED_ART, AP_PLANNED_ART, AP_SHORT_ART, AP_SHORT_ART].to_vec());
        assert!(view.attacks(Some(NetId(1))) && !view.attacks(Some(NetId(9))));
        assert!(covers(&view, Some(NetId(1)), Vec2::new(300.0, 420.0)) && !covers(&view, Some(NetId(1)), Vec2::new(300.0, 300.0)));
    }
}
