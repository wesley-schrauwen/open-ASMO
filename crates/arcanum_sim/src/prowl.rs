//! Prowling: sneaking with the Prowling skill (Arcanum.exe toggle 0x57a0a0 case 0 → 0x45ee30 →
//! 0x45ee90; the skill's value 0x4c62e0; hearing 0x4af470).
//!
//! - **On and off:** the skills view's Prowling button toggles it (`PlayerIntent::Prowl`); a
//!   right click in the world turns it off (0x54de50). The original keeps it in `critter_flags`
//!   (0x1 | 0x2) and switches the critter's art to animation 5 ([`ANIM_PROWL`], 0x45efa0;
//!   off: animation 0): here the PC carries [`Prowling`], its [`crate::Appearance`] takes
//!   animation 5 (later players see it in the `Spawn`), and every player gets
//!   [`LedgerDelta::Prowling`]. It is not saved (builder's call).
//! - **Its value** ([`prowl_value`], 0x4c62e0 skill 6 → 0x4c6410): opposed by each watcher's
//!   Perception (level − PE: 100 at 10 or more, 0 at −10 or less, else table 0x5b7068 / 4), + 10
//!   at Intelligence 20.
//! - **Hearing** ([`hearing_reach`], 0x4af470): a listener hears within `(clamp(PE - 4, 1, 10) +
//!   6) / 2` tiles; against a prowler its Perception first loses the prowler's value in percent
//!   (at most 100). The difficulty 0x4c8430 takes off (flag 0x2000) is not counted.
//!
//! - **Sight** ([`crate::combat::sees`], 0x4af260): within `clamp(PE, 1, 10)` tiles, Perception
//!   cut by a prowler's value in percent; in the watcher's facing cone; nothing between.
//!
//! Not built: invisibility, the light and the difficulty terms of a prowler's value (0x4c8430),
//! dropping prowling when alerted, followers copying their leader, untrained
//! prowlers' path flag 0x200, the hover text's prowling state.

use bevy_ecs::prelude::*;

use crate::{Appearance, LedgerDelta, NetId, Outbox, PlayerControlled, PlayerId, Session};

/// The Prowling skill.
pub const SKILL_PROWLING: usize = 6;
/// The critter animation of a prowler (0x45efa0).
pub const ANIM_PROWL: u32 = 5;

/// The PC is prowling.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prowling;

/// Prowl requests taken from the inbox: the player and on or off.
#[derive(Resource, Debug, Default)]
pub struct ProwlRequests(pub Vec<(PlayerId, bool)>);

/// A prowler: its Prowling level and Intelligence, and what being seen costs it in the light
/// ([`look_difficulty`]: sight only, 0x4af39e).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Prowler {
    pub level: i32,
    pub intelligence: i32,
    /// Against a watcher without dark sight; [`Self::for_watcher`] sets it for one with it.
    pub look: i32,
    /// The luminance where it stands, and its Prowling rank.
    pub light: Option<i32>,
    pub rank: u8,
    /// Its injuries (`critter_flags` 0x80 blinded, 0x400 lame: + 30 each, 0x4c8b0e / 0x4c8b72).
    pub injuries: i32,
    /// The light it bears ([`crate::light::borne_light`]): a lantern gives it away.
    pub borne: i32,
    /// The watcher's helpless amount (0x4c8603 with table bit 0x8: the watcher's state, as a
    /// target's, [`crate::combat::helpless_by_state`]), and the cover on the line between.
    pub watcher_helpless: i32,
    pub cover: i32,
}

impl Prowler {
    /// PC `sheet` prowling on tile `at` of `map` at `hour`.
    /// `borne`: the light it bears ([`crate::Inventory::borne_light`]).
    pub fn of(rules: &arcanum_rules::RulesData, sheet: &arcanum_rules::CharacterSheet, map: &crate::LoadedMap, at: arcanum_formats::map::TileLoc, hour: u32, borne: i32) -> Self {
        let light = map.light_at(at, hour).map(crate::light::luminance);
        let rank = sheet.training[SKILL_PROWLING];
        Self { level: rules.skill_level(sheet, SKILL_PROWLING), intelligence: rules.stats(sheet)[4], look: light.map_or(0, |l| look_difficulty(rank, l, false, borne)), light, rank, injuries: sheet.injuries, borne, ..Default::default() }
    }

    /// With the watcher's helpless amount.
    pub fn watched_by(self, helpless: i32) -> Self {
        Self { watcher_helpless: helpless, ..self }
    }

    /// With `cover` on the line from the watcher.
    pub fn with_cover(self, cover: i32) -> Self {
        Self { cover, ..self }
    }

    /// The look's whole difficulty (0x4c8430 for skill 6 with flag 0x4000): the light
    /// ([`Self::look`]), its injuries, less the watcher's helplessness; then the line (0x4c8be9):
    /// less the cover on it, or for an open line 50 more below Expert (0x4c8bf4).
    pub fn look_total(self) -> i32 {
        let injuries = 30 * i32::from(self.injuries & 0x80 != 0) + 30 * i32::from(self.injuries & 0x400 != 0);
        let line = if self.cover == 0 && self.rank < 2 { 50 } else { -self.cover };
        self.look + injuries - self.watcher_helpless + line
    }

    /// As seen by a watcher that has dark sight or not ([`look_difficulty`]).
    pub fn for_watcher(self, dark_sight: bool) -> Self {
        Self { look: self.light.map_or(0, |l| look_difficulty(self.rank, l, dark_sight, self.borne)), ..self }
    }

    /// Its value against a watcher's eyes of Perception `pe`: [`Self::against`] less
    /// [`Self::look`], in 0-100 (0x4af3b9).
    pub fn seen_against(self, pe: i32) -> i32 {
        (self.against(pe) - self.look_total()).clamp(0, 100)
    }
}

/// What a prowler's own light `l` costs it against a watcher's eyes (0x4af260 → 0x4c8430 with
/// flag 0x4000: L the prowler's lit colour, 0x4c899b): none from Apprentice (0x4c8982); else the
/// dark's term turned round for a watcher without dark sight (0x4c8a84-0x4c8aa4: `30 − p`), so
/// `30 × L / 255`: the brighter the prowler stands, the less it hides; for a watcher with dark
/// sight (0x45fb90 true at 0x4c8a9b) the plain term, so the dark gives it away. The light it
/// bears (`borne`) cuts the term first ([`crate::light::borne_scale`]), so a lit lantern turns
/// it round to near 30.
pub fn look_difficulty(rank: u8, l: i32, dark_sight: bool, borne: i32) -> i32 {
    let p = crate::light::borne_scale(crate::light::dark_difficulty(l), borne);
    if rank >= 1 { 0 } else if dark_sight { p } else { (30 - p).max(0) }
}

impl Prowler {
    /// Its value against a watcher of Perception `pe` ([`prowl_value`]).
    pub fn against(self, pe: i32) -> i32 {
        prowl_value(self.level, self.intelligence, pe)
    }
}

/// The Prowling value of a prowler of skill `level` and `intelligence` against a watcher of
/// Perception `pe` (0x4c62e0 → 0x4c6410: Prowling is opposed, flags 0x298 at 0x5b6f64 with
/// 0x10, by the watcher's Perception; the level-by-five table entry is not used), + 10 at
/// Intelligence 20 (0x4c6369); not capped at 95 (no 0x800).
pub fn prowl_value(level: i32, intelligence: i32, pe: i32) -> i32 {
    crate::pickpocket::opposed_chance(level - pe) + crate::bright_bonus(intelligence)
}

/// How far a listener of `perception` hears, in tiles; `prowler` is the target when it prowls
/// (0x4af470 with noise 1).
pub fn hearing_reach(perception: i32, prowler: Option<Prowler>) -> i64 {
    noise_reach(perception, prowler, NOISE_STEP)
}

/// The noise table (0x5b50c0): what a sound of each loudness adds over 0 (0x4af5e4-0x4af5f2).
pub const NOISES: [i32; 4] = [2, 8, 15, 1];
/// Noise 0, quiet (a blade in the back, 0x4b2319), and 1, a step (noticing, 0x4c86c9).
pub const NOISE_QUIET: usize = 0;
pub const NOISE_STEP: usize = 1;

/// How far a listener of `perception` hears a sound of `noise` ([`NOISES`]): `(clamp(PE' − 4, 1,
/// 10) + table[noise] − table[0]) / 2` tiles (0x4af470).
pub fn noise_reach(perception: i32, prowler: Option<Prowler>, noise: usize) -> i64 {
    let mut perception = perception;
    if let Some(p) = prowler {
        perception -= perception * p.against(perception).clamp(0, 100) / 100;
    }
    i64::from(((perception - 4).clamp(1, 10) + NOISES[noise] - NOISES[0]) / 2)
}

/// Turns prowling on or off as asked; every player in the world is told.
pub fn apply_prowling(mut commands: Commands, mut requests: ResMut<ProwlRequests>, session: Res<Session>, mut outbox: ResMut<Outbox>, mut pcs: Query<(Entity, &NetId, &PlayerControlled, &mut Appearance, Has<Prowling>)>) {
    for (player, on) in std::mem::take(&mut requests.0) {
        let Some((entity, &net_id, _, mut look, prowling)) = pcs.iter_mut().find(|pc| pc.2.0 == player) else { continue };
        if prowling == on {
            continue;
        }
        look.art = crate::gait::with_anim(look.art, if on { ANIM_PROWL } else { arcanum_formats::art_id::ANIM_STAND });
        if on {
            commands.entity(entity).insert(Prowling);
        } else {
            commands.entity(entity).remove::<Prowling>();
        }
        crate::ground::broadcast(&session, &mut outbox, LedgerDelta::Prowling { net_id, on });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prowlers_cut_the_listeners_perception() {
        assert_eq!((prowl_value(0, 8, 8), prowl_value(8, 8, 8), prowl_value(18, 8, 8), prowl_value(0, 8, 18)), (16, 47, 100, 0), "opposed by Perception");
        assert_eq!(prowl_value(8, 20, 8), 57, "Intelligence 20: + 10");
        // A prowler's own light against eyes (0x4c8a84): noon 30, the default night 10, pitch dark 0.
        assert_eq!((look_difficulty(0, 254, false, 0), look_difficulty(0, 79, false, 0), look_difficulty(0, 0, false, 0), look_difficulty(1, 254, false, 0)), (30, 10, 0, 0));
        assert_eq!(look_difficulty(0, 0, false, crate::light::OCF_LIGHT_XLARGE), 23, "a lantern in the dark: 30 - 30 / 4");
        assert_eq!((look_difficulty(0, 0, true, 0), look_difficulty(0, 254, true, 0)), (30, 0), "a dark-seeing watcher: the dark gives it away");
        let p = Prowler { light: Some(0), ..Default::default() };
        assert_eq!((p.for_watcher(false).look, p.for_watcher(true).look), (0, 30));
        // The rest of the look (0x4c8aed-0x4c8bf7): in the open below Expert 50 more; cover takes
        // its cost off; blind or lame 30 more each; a paralyzed watcher 50 less.
        let plain = Prowler { level: 8, intelligence: 8, ..Default::default() };
        assert_eq!((plain.look_total(), plain.with_cover(20).look_total(), Prowler { rank: 2, ..plain }.look_total()), (50, -20, 0));
        assert_eq!(Prowler { injuries: 0x480, ..plain }.with_cover(20).look_total(), 40);
        assert_eq!(plain.watched_by(50).look_total(), 0);
        assert_eq!((plain.seen_against(8), plain.with_cover(20).seen_against(8)), (0, 67), "seen in the open, hidden behind a stone");
        let lit = Prowler { level: 8, intelligence: 8, look: 30, rank: 2, ..Default::default() };
        assert_eq!((lit.against(8), lit.seen_against(8)), (47, 17), "seen: the light takes 30 off");
        assert_eq!(hearing_reach(10, None), 6, "(6 + 6) / 2");
        assert_eq!(hearing_reach(10, Some(Prowler { level: 11, intelligence: 8, rank: 2, ..Default::default() })), 3, "level 11 against PE 10: 52, PE 5: (1 + 6) / 2");
        assert_eq!(hearing_reach(8, Some(Prowler { level: 20, intelligence: 20, rank: 2, ..Default::default() })), 3, "110: at most all of it");
    }
}
