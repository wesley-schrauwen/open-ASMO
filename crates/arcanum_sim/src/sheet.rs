//! A PC's sheet in the world: the character screen spends its points through the host.
//!
//! As the original's in-game character screen does (Arcanum.exe 0x5597c0, mode 1):
//! - Opening it takes a snapshot ([`SheetRequest::Open`]): the sheet's draft is folded in, so
//!   minus can lower a value back down to what it was at opening, and no further (charedit 19).
//! - Raising a stat, buying hit points or fatigue or a skill point takes effect at once: derived stats such as
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

use crate::{
    Appearance, Gait, LedgerDelta, NetId, Outbox, PartyPace, PlayerControlled, PlayerId, Session,
    Sheet, SimRules, SimVfs, WorldState, pc_status,
};

/// A request about the player's own PC's sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SheetRequest {
    /// The character screen opened: later changes can be undone back to here.
    Open,
    /// A stat raised or lowered, or a point put into or taken out of hit points, fatigue or a
    /// skill.
    Action(CharacterAction),
    /// The character screen closed: the changes stay.
    Close,
}

/// Applies `request` to `sheet` with the shared rules. Only point spending (stats, pools,
/// skills, tech degrees, spells) is allowed in the world.
pub fn apply_sheet_request(
    rules: &RulesData,
    sheet: &CharacterSheet,
    request: &SheetRequest,
) -> Result<CharacterSheet, Refusal> {
    match request {
        SheetRequest::Open | SheetRequest::Close => Ok(rules.accept(sheet)),
        SheetRequest::Action(
            action @ (CharacterAction::RaiseStat(_)
            | CharacterAction::LowerStat(_)
            | CharacterAction::AdjustPool(..)
            | CharacterAction::RaiseSkill(_)
            | CharacterAction::LowerSkill(_)
            | CharacterAction::RaiseTech(_)
            | CharacterAction::LowerTech(_)
            | CharacterAction::RaiseSpell(_)
            | CharacterAction::LowerSpell(_)),
        ) => rules.apply(sheet, action),
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
    mut pcs: Query<(
        &NetId,
        &PlayerControlled,
        &mut Sheet,
        &mut Appearance,
        &mut Gait,
    )>,
) {
    for (player, seq, request) in requests.0.drain(..) {
        let Some((&net_id, _, mut sheet, mut look, mut gait)) =
            pcs.iter_mut().find(|(_, p, ..)| p.0 == player)
        else {
            continue;
        };
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
                    outbox.0.push((
                        other,
                        LedgerDelta::PcStatus {
                            net_id,
                            status: status.clone(),
                        },
                    ));
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
        outbox.0.push((
            player,
            LedgerDelta::SheetUpdate {
                ack: seq,
                sheet: sheet.0.clone(),
            },
        ));
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
        CharacterSheet {
            unspent: points,
            ..rules().new_sheet()
        }
    }

    #[test]
    fn changes_can_be_undone_until_the_screen_closes() {
        let r = rules();
        let s = apply_sheet_request(&r, &with_points(2), &SheetRequest::Open).unwrap();
        let raised =
            apply_sheet_request(&r, &s, &SheetRequest::Action(CharacterAction::RaiseStat(0)))
                .unwrap();
        assert_eq!(r.stats(&raised)[0], r.stats(&s)[0] + 1, "at once");
        let back = apply_sheet_request(
            &r,
            &raised,
            &SheetRequest::Action(CharacterAction::LowerStat(0)),
        )
        .unwrap();
        assert_eq!(r.stats(&back), r.stats(&s), "back to the values at opening");
        assert_eq!(
            apply_sheet_request(
                &r,
                &back,
                &SheetRequest::Action(CharacterAction::LowerStat(0))
            ),
            Err(Refusal::BelowAccepted),
            "and no further"
        );
        let closed = apply_sheet_request(&r, &raised, &SheetRequest::Close).unwrap();
        assert_eq!(
            apply_sheet_request(
                &r,
                &closed,
                &SheetRequest::Action(CharacterAction::LowerStat(0))
            ),
            Err(Refusal::BelowAccepted),
            "closing keeps it"
        );
        let more_hp = apply_sheet_request(
            &r,
            &closed,
            &SheetRequest::Action(CharacterAction::AdjustPool(Pool::HitPoints, 1)),
        )
        .unwrap();
        assert_eq!(
            r.derived(&more_hp).hit_points,
            r.derived(&closed).hit_points + 4
        );
    }

    #[test]
    fn sheets_saved_before_skills_still_load() {
        let mut json = serde_json::to_value(with_points(2)).unwrap();
        for key in ["skills", "training"] {
            json.as_object_mut().unwrap().remove(key);
        }
        json["draft"].as_object_mut().unwrap().remove("skills");
        let sheet: CharacterSheet = serde_json::from_value(json).unwrap();
        assert_eq!((sheet.skills, sheet.draft.skills), ([0; 16], [0; 16]));
    }

    #[test]
    fn skills_change_in_the_world() {
        let r = rules();
        let s = apply_sheet_request(
            &r,
            &with_points(2),
            &SheetRequest::Action(CharacterAction::RaiseSkill(1)),
        )
        .unwrap();
        assert_eq!(r.skill_level(&s, 1), 4);
    }

    #[test]
    fn only_stats_pools_and_skills_change_in_the_world() {
        let r = rules();
        for action in [
            CharacterAction::SetName("X".into()),
            CharacterAction::StepRace(1),
            CharacterAction::ToggleGender,
            CharacterAction::Accept,
        ] {
            assert_eq!(
                apply_sheet_request(&r, &with_points(2), &SheetRequest::Action(action.clone())),
                Err(Refusal::NotAllowed),
                "{action:?}"
            );
        }
    }
}

/// Experience earned this tick, to share among the team ([`share_experience`]).
#[derive(Resource, Debug, Default)]
pub struct ExperienceAwards(pub Vec<i32>);

/// Each PC's share of `amount` among `members` (0x45f110): an equal part, 1 when it rounds to 0.
pub fn experience_share(amount: i32, members: usize) -> i32 {
    let Ok(members) = i32::try_from(members) else {
        return 0;
    };
    if members == 0 {
        return 0;
    }
    match amount / members {
        0 => 1,
        share => share,
    }
}

/// Shares the experience earned among every PC in the world (owner's rule: the whole party is
/// one team, as the original's multiplayer splits it among the living members of the team,
/// 0x45f110). Each share goes through the PC's `xpgain` effects and level-ups; each PC that
/// rises hears "Congratulations! You are now level %d." (`mes/intgame.mes` 11).
#[allow(clippy::too_many_arguments)]
pub fn share_experience(
    mut awards: ResMut<ExperienceAwards>,
    mut world: ResMut<WorldState>,
    rules: Res<SimRules>,
    session: Res<Session>,
    vfs: Option<Res<SimVfs>>,
    mut outbox: ResMut<Outbox>,
    mut pcs: Query<(&NetId, &PlayerControlled, &mut Sheet)>,
) {
    if awards.0.is_empty() {
        return;
    }
    let members: Vec<PlayerId> = session
        .players
        .iter()
        .filter(|(_, s)| matches!(s, crate::SessionPlayer::InWorld(_)))
        .map(|(&p, _)| p)
        .collect();
    let level_up = vfs
        .as_ref()
        .and_then(|v| v.0.read().ok())
        .and_then(|v| v.read("mes/intgame.mes").ok())
        .and_then(|d| arcanum_formats::mes::MesFile::parse(&d).ok())
        .and_then(|m| m.get(11).map(str::to_string));
    for amount in std::mem::take(&mut awards.0) {
        let share = experience_share(amount, members.len());
        for (&net_id, controlled, mut sheet) in &mut pcs {
            if !members.contains(&controlled.0) {
                continue;
            }
            let levels = rules.0.gain_experience(&mut sheet.0, share);
            if let Some(saved) = world
                .0
                .as_mut()
                .and_then(|w| w.players.get_mut(&controlled.0))
            {
                saved.blueprint.sheet = sheet.0.clone();
            }
            outbox.0.push((
                controlled.0,
                LedgerDelta::SheetUpdate {
                    ack: 0,
                    sheet: sheet.0.clone(),
                },
            ));
            for level in &levels {
                if let Some(text) = &level_up {
                    outbox.0.push((
                        controlled.0,
                        LedgerDelta::ScriptMessage {
                            text: text.replace("%d", &level.to_string()),
                        },
                    ));
                }
            }
            crate::ground::broadcast(
                &session,
                &mut outbox,
                LedgerDelta::PcStatus {
                    net_id,
                    status: pc_status(&rules.0, &sheet.0),
                },
            );
        }
    }
}

/// The fate options (`mes/fate_ui.mes` 0-11).
pub const FATE_OPTIONS: usize = 12;
/// Each option's bit in the waiting uses (table 0x5bc48c); option 0 happens at once.
pub fn fate_bit(option: u8) -> u16 {
    match option {
        1..=11 => 1 << (option - 1),
        _ => 0,
    }
}
/// "Force good reaction": the next reaction computed is 100 (0x4c1020).
pub const FATE_GOOD_REACTION: u8 = 1;

/// Spends a fate point on `option` (0x4f50c0): none left, or that use already waiting:
/// refused. "Full heal now" (0) heals at once (0x4f52d0: hit points, poison, fatigue); the others
/// wait for their moment (attack, spell, skill, reaction). Returns whether it was spent.
pub fn spend_fate(sheet: &mut CharacterSheet, option: u8) -> bool {
    if usize::from(option) >= FATE_OPTIONS || sheet.fate_points < 1 {
        return false;
    }
    let bit = fate_bit(option);
    if sheet.fate_pending & bit != 0 && bit != 0 {
        return false;
    }
    sheet.fate_points -= 1;
    if bit == 0 {
        sheet.hp_damage = 0;
        sheet.poison = 0;
        sheet.fatigue_damage = 0;
    } else {
        sheet.fate_pending |= bit;
    }
    true
}

/// Takes a waiting use back, with its point (0x4f51b0).
pub fn refund_fate(sheet: &mut CharacterSheet, option: u8) -> bool {
    let bit = fate_bit(option);
    if bit == 0 || sheet.fate_pending & bit == 0 {
        return false;
    }
    sheet.fate_pending &= !bit;
    sheet.fate_points += 1;
    true
}

/// Uses up a waiting fate use if it is there (0x4f5270).
pub fn consume_fate(sheet: &mut CharacterSheet, option: u8) -> bool {
    let bit = fate_bit(option);
    if bit == 0 || sheet.fate_pending & bit == 0 {
        return false;
    }
    sheet.fate_pending &= !bit;
    true
}

/// Fate requests taken from the inbox: player, option, spend (else take back).
#[derive(Resource, Debug, Default)]
pub struct FateRequests(pub Vec<(PlayerId, u8, bool)>);

/// Spends or takes back fate points (the host decides, as 0x49e8d3 does for packet 0x24); the
/// player gets the sheet, everyone the PC's gauges.
pub fn apply_fate(
    mut requests: ResMut<FateRequests>,
    mut world: ResMut<WorldState>,
    rules: Res<SimRules>,
    session: Res<Session>,
    mut outbox: ResMut<Outbox>,
    mut pcs: Query<(&NetId, &PlayerControlled, &mut Sheet)>,
) {
    for (player, option, on) in std::mem::take(&mut requests.0) {
        let Some((&net_id, _, mut sheet)) = pcs.iter_mut().find(|pc| pc.1.0 == player) else {
            continue;
        };
        let changed = if on {
            spend_fate(&mut sheet.0, option)
        } else {
            refund_fate(&mut sheet.0, option)
        };
        if !changed {
            continue;
        }
        if let Some(saved) = world.0.as_mut().and_then(|w| w.players.get_mut(&player)) {
            saved.blueprint.sheet = sheet.0.clone();
        }
        outbox.0.push((
            player,
            LedgerDelta::SheetUpdate {
                ack: 0,
                sheet: sheet.0.clone(),
            },
        ));
        crate::ground::broadcast(
            &session,
            &mut outbox,
            LedgerDelta::PcStatus {
                net_id,
                status: pc_status(&rules.0, &sheet.0),
            },
        );
    }
}

#[cfg(test)]
mod fate_tests {
    use super::*;

    #[test]
    fn fate_points_buy_a_heal_now_or_a_use_later_and_come_back_untaken() {
        let mut s = CharacterSheet {
            fate_points: 2,
            hp_damage: 9,
            poison: 4,
            fatigue_damage: 3,
            ..Default::default()
        };
        assert!(spend_fate(&mut s, 0));
        assert_eq!(
            (
                s.fate_points,
                s.hp_damage,
                s.poison,
                s.fatigue_damage,
                s.fate_pending
            ),
            (1, 0, 0, 0, 0)
        );
        assert!(spend_fate(&mut s, 2));
        assert_eq!(
            (s.fate_points, s.fate_pending),
            (0, 0x2),
            "critical success on the next attack waits"
        );
        assert!(!spend_fate(&mut s, 3), "no points left");
        assert!(refund_fate(&mut s, 2) && s.fate_points == 1 && s.fate_pending == 0);
        assert!(!refund_fate(&mut s, 2), "nothing waiting");
        assert!(
            spend_fate(&mut s, FATE_GOOD_REACTION)
                && consume_fate(&mut s, FATE_GOOD_REACTION)
                && s.fate_pending == 0
        );
        assert_eq!(fate_bit(11), 0x400);
    }
}
