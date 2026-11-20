//! A PC's sheet in the world: the character screen spends its points through the host.
//!
//! As the original's in-game character screen does (Arcanum.exe 0x5597c0, mode 1):
//! - Opening it takes a snapshot ([`SheetRequest::Open`]): the sheet's draft is folded in, so
//!   minus can lower a value back down to what it was at opening, and no further (charedit 19).
//! - Raising a stat or buying hit points or fatigue takes effect at once: derived stats such as
//!   the maximum hit points follow.
//! - Closing it ([`SheetRequest::Close`]) keeps every change; there is no cancel.
//! - The game does not pause. In multiplayer the original sends each change to the host
//!   (message 0x7f); here every change goes through the host.
//!
//! The host answers each request with a [`LedgerDelta::SheetUpdate`] numbered like it, refused
//! or not, tells every player the PC's new gauges ([`LedgerDelta::PcStatus`]) and, if its speed
//! changed, its new speed ([`LedgerDelta::Speed`]). The save keeps the sheet.

use arcanum_rules::{CharacterAction, CharacterSheet, Refusal, RulesData};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{Appearance, Gait, LedgerDelta, NetId, Outbox, PartyPace, PlayerControlled, PlayerId, Session, Sheet, SimRules, SimVfs, WorldState, pc_status};

/// A request about the player's own PC's sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SheetRequest {
    /// The character screen opened: later changes can be undone back to here.
    Open,
    /// A stat raised or lowered, or a point put into or taken out of hit points or fatigue.
    Action(CharacterAction),
    /// The character screen closed: the changes stay.
    Close,
}

/// Applies `request` to `sheet` with the shared rules. Only stat and pool changes are allowed
/// in the world.
pub fn apply_sheet_request(rules: &RulesData, sheet: &CharacterSheet, request: &SheetRequest) -> Result<CharacterSheet, Refusal> {
    match request {
        SheetRequest::Open | SheetRequest::Close => Ok(rules.accept(sheet)),
        SheetRequest::Action(action @ (CharacterAction::RaiseStat(_) | CharacterAction::LowerStat(_) | CharacterAction::AdjustPool(..))) => rules.apply(sheet, action),
        SheetRequest::Action(_) => Err(Refusal::NotAllowed),
    }
}

/// Sheet requests received, for [`apply_sheet_requests`].
#[derive(Resource, Debug, Default)]
pub struct SheetRequests(pub Vec<(PlayerId, u32, SheetRequest)>);

/// Applies the [`SheetRequests`] to the players' PCs: see the module docs.
#[allow(clippy::too_many_arguments)]
pub fn apply_sheet_requests(
    mut requests: ResMut<SheetRequests>,
    rules: Res<SimRules>,
    session: Res<Session>,
    vfs: Option<Res<SimVfs>>,
    mut world: ResMut<WorldState>,
    mut outbox: ResMut<Outbox>,
    mut pace: ResMut<PartyPace>,
    mut pcs: Query<(&NetId, &PlayerControlled, &mut Sheet, &mut Appearance, &mut Gait)>,
) {
    for (player, seq, request) in requests.0.drain(..) {
        let Some((&net_id, _, mut sheet, mut look, mut gait)) = pcs.iter_mut().find(|(_, p, ..)| p.0 == player) else { continue };
        if let Ok(changed) = apply_sheet_request(&rules.0, &sheet.0, &request)
            && changed != sheet.0
        {
            let before = pc_status(&rules.0, &sheet.0);
            sheet.0 = changed;
            if let Some(saved) = world.0.as_mut().and_then(|w| w.players.get_mut(&player)) {
                saved.blueprint.sheet = sheet.0.clone();
            }
            let status = pc_status(&rules.0, &sheet.0);
            if status != before {
                for &other in session.players.keys() {
                    outbox.0.push((other, LedgerDelta::PcStatus { net_id, status: status.clone() }));
                }
            }
            let speed = rules.0.derived(&sheet.0).speed;
            if speed != look.speed {
                look.speed = speed;
                let vfs_guard = vfs.as_ref().and_then(|v| v.0.read().ok());
                let vfs = vfs_guard.as_deref();
                *gait = Gait::load(look.art, speed, |p| vfs.and_then(|v| v.read(p).ok()));
                for &other in session.players.keys() {
                    outbox.0.push((other, LedgerDelta::Speed { net_id, speed }));
                }
                // The leader sets the party's pace: work it out again.
                if pace.net_id == Some(net_id) {
                    pace.net_id = None;
                }
            }
        }
        outbox.0.push((player, LedgerDelta::SheetUpdate { ack: seq, sheet: sheet.0.clone() }));
    }
}

#[cfg(test)]
mod tests {
    use arcanum_rules::Pool;

    use super::*;

    fn rules() -> RulesData {
        RulesData::default()
    }

    fn with_points(points: i32) -> CharacterSheet {
        CharacterSheet { unspent: points, ..rules().new_sheet() }
    }

    #[test]
    fn changes_can_be_undone_until_the_screen_closes() {
        let r = rules();
        let s = apply_sheet_request(&r, &with_points(2), &SheetRequest::Open).unwrap();
        let raised = apply_sheet_request(&r, &s, &SheetRequest::Action(CharacterAction::RaiseStat(0))).unwrap();
        assert_eq!(r.stats(&raised)[0], r.stats(&s)[0] + 1, "at once");
        let back = apply_sheet_request(&r, &raised, &SheetRequest::Action(CharacterAction::LowerStat(0))).unwrap();
        assert_eq!(r.stats(&back), r.stats(&s), "back to the values at opening");
        assert_eq!(apply_sheet_request(&r, &back, &SheetRequest::Action(CharacterAction::LowerStat(0))), Err(Refusal::BelowAccepted), "and no further");
        let closed = apply_sheet_request(&r, &raised, &SheetRequest::Close).unwrap();
        assert_eq!(apply_sheet_request(&r, &closed, &SheetRequest::Action(CharacterAction::LowerStat(0))), Err(Refusal::BelowAccepted), "closing keeps it");
        let more_hp = apply_sheet_request(&r, &closed, &SheetRequest::Action(CharacterAction::AdjustPool(Pool::HitPoints, 1))).unwrap();
        assert_eq!(r.derived(&more_hp).hit_points, r.derived(&closed).hit_points + 4);
    }

    #[test]
    fn only_stats_and_pools_change_in_the_world() {
        let r = rules();
        for action in [CharacterAction::SetName("X".into()), CharacterAction::StepRace(1), CharacterAction::ToggleGender, CharacterAction::Accept] {
            assert_eq!(apply_sheet_request(&r, &with_points(2), &SheetRequest::Action(action.clone())), Err(Refusal::NotAllowed), "{action:?}");
        }
    }
}
