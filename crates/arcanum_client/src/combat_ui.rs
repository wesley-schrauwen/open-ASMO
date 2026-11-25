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
//!   "Critical miss" over the attacker; white over NPCs, red over PCs.
//!
//! Not built: the orange and red lights of a planned move's cost, the battle cursor (art 353) and
//! `hoverattack.art`, the attack, hit and death animations (attacks most likely 20 / 21), called
//! shots, the button's pressed look.

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

/// The fight's deltas from the bridge, for [`apply_combat_deltas`].
#[derive(Resource, Debug, Default)]
pub struct CombatDeltas(pub Vec<LedgerDelta>);

/// Applies the fight's deltas (`mes/combat.mes` read on first use).
fn apply_combat_deltas(mut deltas: ResMut<CombatDeltas>, mut view: ResMut<CombatView>, mut floats: Option<ResMut<crate::dialogue_ui::Floats>>, party: Res<crate::actors::Party>, vfs: Option<Res<VfsResource>>, mut mes: Local<Option<arcanum_formats::mes::MesFile>>) {
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
        apply(&delta, &mut view, floats.as_deref_mut(), &is_pc, &text);
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
            .init_resource::<CombatDeltas>()
            .add_systems(OnExit(AppState::InGame), |mut v: ResMut<CombatView>| *v = CombatView::default())
            .add_observer(on_combat_button)
            .add_systems(Update, (apply_combat_deltas, show_ap_bar, end_turn).chain().run_if(in_state(AppState::InGame)));
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
fn show_ap_bar(mut commands: Commands, view: Res<CombatView>, local: Res<LocalPc>, roots: Query<Entity, With<ApBar>>, assets: Res<AssetServer>, vfs: Option<Res<VfsResource>>, mut art: Local<InterfaceArt>, mut shown: Local<Option<(bool, i32)>>) {
    let pc = local.0.map(|(id, _)| id);
    let want = view.fighting(pc).then(|| (view.turn == pc, view.ap));
    if *shown == want && !(want.is_some() && roots.is_empty()) {
        return;
    }
    *shown = want;
    for root in &roots {
        commands.entity(root).despawn();
    }
    let Some((mine, ap)) = want else { return };
    let vfs = vfs.as_deref();
    let (Some(bar), Some(size)) = (art.image(AP_BAR_ART, &assets, vfs), art.size(AP_BAR_ART, vfs)) else { return };
    let end = art.image(END_TURN_ART, &assets, vfs);
    let end_size = art.size(END_TURN_ART, vfs).unwrap_or(Vec2::new(40.0, 20.0));
    let light = art.image(AP_LIGHT_ART, &assets, vfs);
    let light_size = art.size(AP_LIGHT_ART, vfs).unwrap_or(Vec2::splat(10.0));
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
            if mine && let Some(light) = light {
                for x in AP_LIGHT_X.iter().take(ap.clamp(0, 20) as usize) {
                    screen.spawn((bevy::picking::Pickable::IGNORE, ImageNode::new(light.clone()), place(AP_BAR_AT + Vec2::new(*x, AP_LIGHT_Y), light_size)));
                }
            }
        });
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
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attacks_float_their_combat_lines() {
        let mes = |k: u32| match k {
            1 => "HT".to_string(),
            10 => "FT".into(),
            11 => "Dodge!".into(),
            12 => "Critical hit".into(),
            14 => "Damage +100%".into(),
            _ => "?".into(),
        };
        let hit = arcanum_sim::combat::AttackOutcome { hit: true, critical: true, multiplier: 200, hp: 8, fatigue: 3, ..Default::default() };
        let floats = attack_floats(NetId(1), NetId(2), &hit, true, &mes);
        let texts: Vec<&str> = floats.iter().map(|f| f.1.as_str()).collect();
        assert_eq!(texts, ["Critical hit", "Damage +100%", "HT 8", "FT 3"]);
        assert!(floats.iter().all(|f| f.0 == NetId(2) && f.2 == FloatColour::Red), "over the PC, red");
        let dodged = arcanum_sim::combat::AttackOutcome { dodged: true, ..Default::default() };
        assert_eq!(attack_floats(NetId(1), NetId(2), &dodged, false, &mes), [(NetId(2), "Dodge!".into(), FloatColour::White)]);
        let view = CombatView { order: vec![NetId(1), NetId(2)], turn: Some(NetId(1)), ap: 7, mode: false };
        assert!(view.attacks(Some(NetId(1))) && !view.attacks(Some(NetId(9))));
        assert!(covers(&view, Some(NetId(1)), Vec2::new(300.0, 420.0)) && !covers(&view, Some(NetId(1)), Vec2::new(300.0, 300.0)));
    }
}
