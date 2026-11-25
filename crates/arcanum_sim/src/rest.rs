//! Sleeping and waiting (Arcanum.exe's sleep interface: open 0x57b180, click 0x57b710, step
//! 0x57b7f0, finish 0x57ba70 / 0x57b450; healing 0x45e910).
//!
//! - **Asking** ([`PlayerIntent::RestWindow`](crate::PlayerIntent::RestWindow), the S key or the
//!   top bar's sleep button): the host checks, in the original's order, and answers with the
//!   window's [`RestMode`] or a refusal (`mes/SleepUI.mes`):
//!   - unconscious: [`ALREADY_UNCONSCIOUS`] ("You are unconscious already.");
//!   - the PC's sector's town map (`townmap_info`): none (the wilderness) is sleeping; a town
//!     whose `Rules/TownMap.mes` line carries `[w:1]` is waiting; any other is refused with
//!     [`CANNOT_WAIT`] ("You cannot wait here.");
//!   - another player in the world: refused without a word (the original has no sleeping in
//!     multiplayer and replaces the button).
//! - **Resting** ([`PlayerIntent::Rest`](crate::PlayerIntent::Rest)): the host checks again,
//!   stops the PC, then steps: the first [`FIRST_STEP_TICKS`] after the click, the rest
//!   [`STEP_TICKS`] apart (50 and 200 real ms). Each step jumps the clock one hour; when
//!   sleeping, it heals the PC ([`heal_at_rest`]: hit point damage by the heal rate, fatigue
//!   damage by three times it). Then it goes on while the [`RestOption`] says so. The jump only
//!   moves the clock: the fatigue recovery due meanwhile catches up on the next ticks
//!   ([`crate::recover_fatigue`]), as the original's time events do.
//! - Meanwhile the player's moves and uses are dropped ([`crate::gate_unconscious`]).
//!   [`PlayerIntent::StopRest`](crate::PlayerIntent::StopRest) ends it early (S again).
//!   Either way the player is sent [`LedgerDelta::RestEnded`](crate::LedgerDelta::RestEnded).
//!
//! - **Beds** (scenery of art category [`BED_CATEGORY`], 0x428c74 → 0x57b180): using one asks
//!   for the window on it ([`BedRequests`]). No town map check; the bed's "use" script runs
//!   (0x441980): running the default refuses with [`CANNOT_SLEEP_HERE`] ("You cannot sleep here.
//!   Find an inn and pay for a bed.": every bed without a script, and an inn's until paid);
//!   a bed another PC rests in refuses with [`BED_OCCUPIED`] (0x45f2d0). The window opens
//!   for sleeping, and each hour in the bed heals twice ([`heal_at_rest`]).
//!
//! The host keeps what the window was opened with ([`RestOffer`]) for the rest that follows.
//!
//! Not built: hiding the PC in the bed and moving it off the bed's tile (0x45f2d0); NPCs in
//! beds (`scenery_whos_in_me`); the combat check ("You cannot sleep with
//! enemies near."); followers resting too; the hit points (nothing damages them yet, so "Until
//! Healed" stops after one hour, as the original does at full health); death ending it.

use std::collections::BTreeSet;

use arcanum_formats::mes::MesFile;
use arcanum_vfs::Vfs;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    LedgerDelta, LoadedMap, Outbox, PlayerControlled, PlayerId, Position, Session, SessionPlayer,
    Sheet, SimRules, WorldState, pc_status,
};

/// `mes/SleepUI.mes`: "You are unconscious already."
pub const ALREADY_UNCONSCIOUS: u32 = 11;
/// `mes/SleepUI.mes`: "You cannot wait here."
pub const CANNOT_WAIT: u32 = 20;
/// `mes/SleepUI.mes`: "You cannot sleep here. Find an inn and pay for a bed."
pub const CANNOT_SLEEP_HERE: u32 = 8;
/// `mes/SleepUI.mes`: "That bed is occupied."
pub const BED_OCCUPIED: u32 = 10;
/// The beds' scenery art category (`art/scenery/scenery.mes` 22000-22011).
pub const BED_CATEGORY: u32 = 22;
/// The town map list, with `[w:1]` on the towns where waiting is allowed.
pub const TOWN_MAP: &str = "Rules/TownMap.mes";
/// Host ticks from the click to the first hour (the original's 50 ms, rounded up).
pub const FIRST_STEP_TICKS: u32 = 2;
/// Host ticks between hours (the original's 200 ms).
pub const STEP_TICKS: u32 = 6;
/// The hour "Until Morning" stops at.
pub const MORNING: u32 = 7;
/// The hour "Until Evening" stops at.
pub const EVENING: u32 = 20;

/// What the window offers: sleeping heals, waiting only passes time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestMode {
    Sleep,
    Wait,
}

impl RestMode {
    /// The window's rows (0x5cb340): sleeping has "Until Healed", waiting does not.
    pub fn options(self) -> &'static [RestOption] {
        let all = &RestOption::ALL;
        match self {
            Self::Sleep => all,
            Self::Wait => &all[..7],
        }
    }

    /// The `mes/SleepUI.mes` line of row `row`.
    pub fn label(self, row: usize) -> u32 {
        match self {
            Self::Sleep => row as u32,
            Self::Wait => row as u32 + 12,
        }
    }
}

/// How long to rest (the window's rows, 0x5cb340: 1, 2, 4, 8, 24, -1, -2, -3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestOption {
    Hours(u32),
    /// Until 7:00.
    UntilMorning,
    /// Until 20:00.
    UntilEvening,
    /// While the PC's hit points are down (fatigue does not count).
    UntilHealed,
}

impl RestOption {
    pub const ALL: [RestOption; 8] = [
        Self::Hours(1),
        Self::Hours(2),
        Self::Hours(4),
        Self::Hours(8),
        Self::Hours(24),
        Self::UntilMorning,
        Self::UntilEvening,
        Self::UntilHealed,
    ];
}

/// Whether `object` is a bed: scenery of art category [`BED_CATEGORY`] (0x428c74).
pub fn is_bed(
    object: &arcanum_formats::object::Object,
    prototypes: &arcanum_formats::object::Prototypes,
) -> bool {
    object.object_type == arcanum_formats::object::TYPE_SCENERY
        && prototypes
            .art_id(object)
            .is_some_and(|a| a.scenery_category() == BED_CATEGORY)
}

/// A PC resting: hours left to go (for [`RestOption::Hours`]), ticks to the next hour, and
/// the bed it rests in.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resting {
    pub mode: RestMode,
    pub option: RestOption,
    pub hours_left: u32,
    pub ticks: u32,
    pub bed: Option<crate::MapObjectId>,
}

/// The sleep window a PC's player was given: in a bed, or not.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestOffer {
    pub mode: RestMode,
    pub bed: Option<crate::MapObjectId>,
}

impl Resting {
    pub fn new(mode: RestMode, option: RestOption) -> Self {
        let hours_left = match option {
            RestOption::Hours(n) => n.max(1),
            _ => 0,
        };
        Self {
            mode,
            option,
            hours_left,
            ticks: FIRST_STEP_TICKS,
            bed: None,
        }
    }

    /// After an hour has passed, ending at `hour` with `hp_damage` left: whether to go on
    /// (0x57b7f0). Every option takes at least one hour.
    pub fn go_on(&mut self, hour: u32, hp_damage: i32) -> bool {
        match self.option {
            RestOption::Hours(_) => {
                self.hours_left = self.hours_left.saturating_sub(1);
                self.hours_left > 0
            }
            RestOption::UntilMorning => hour != MORNING,
            RestOption::UntilEvening => hour != EVENING,
            RestOption::UntilHealed => hp_damage > 0,
        }
    }
}

/// An hour's healing at rest (0x45e910) of `hp` and `fatigue` damage at `heal_rate`, `times`
/// over (2 in a bed): hit points by the rate, fatigue by three times it, neither below 0.
pub fn heal_at_rest(hp: i32, fatigue: i32, heal_rate: i32, times: i32) -> (i32, i32) {
    let heal = |damage: i32, amount: i32| {
        if damage > 0 {
            (damage - amount).max(0)
        } else {
            damage
        }
    };
    (
        heal(hp, heal_rate * times),
        heal(fatigue, 3 * heal_rate * times),
    )
}

/// The town map entries where waiting is allowed: lines with `[w:1]` or `[W:1]` (0x4be1c0).
pub fn waiting_towns(mes: &MesFile) -> BTreeSet<u32> {
    mes.messages
        .iter()
        .filter(|(_, text)| text.contains("[w:1]") || text.contains("[W:1]"))
        .map(|(&id, _)| id)
        .collect()
}

/// Reads [`TOWN_MAP`]'s waiting towns; none when it cannot be read.
pub fn load_waiting_towns(vfs: &Vfs) -> BTreeSet<u32> {
    vfs.read(TOWN_MAP)
        .ok()
        .and_then(|d| MesFile::parse(&d).ok())
        .map(|m| waiting_towns(&m))
        .unwrap_or_default()
}

/// What the window may offer in a sector with town map `townmap` (0 for none): `Err` is the
/// refusal (0x57b180).
pub fn rest_mode(townmap: u32, waiting: &BTreeSet<u32>) -> Result<RestMode, u32> {
    match townmap {
        0 => Ok(RestMode::Sleep),
        town if waiting.contains(&town) => Ok(RestMode::Wait),
        _ => Err(CANNOT_WAIT),
    }
}

/// Requests taken from the inbox: `None` asks for the window, `Some` rests.
#[derive(Resource, Debug, Default)]
pub struct RestRequests(pub Vec<(PlayerId, Option<RestOption>)>);

/// Players who used a bed: they ask for the window on it.
#[derive(Resource, Debug, Default)]
pub struct BedRequests(pub Vec<(PlayerId, crate::MapObjectId)>);

/// Players who asked to stop resting.
#[derive(Resource, Debug, Default)]
pub struct StopRestRequests(pub Vec<PlayerId>);

/// The towns where waiting is allowed, read once from the game data.
#[derive(Resource, Debug, Default)]
pub struct WaitingTowns(pub Option<BTreeSet<u32>>);

type RestingPc<'a> = (
    Entity,
    &'a PlayerControlled,
    &'a Position,
    &'a Sheet,
    Has<crate::Walking>,
    Option<&'a Resting>,
    Option<&'a RestOffer>,
);

/// Answers the requests for the window and to rest: see the module docs.
#[allow(clippy::too_many_arguments)]
pub fn start_resting(
    mut commands: Commands,
    mut requests: ResMut<RestRequests>,
    mut stops: ResMut<StopRestRequests>,
    mut towns: ResMut<WaitingTowns>,
    vfs: Option<Res<crate::SimVfs>>,
    mut map: ResMut<LoadedMap>,
    session: Res<Session>,
    rules: Res<SimRules>,
    mut world: ResMut<WorldState>,
    mut outbox: ResMut<Outbox>,
    pcs: Query<RestingPc>,
    (mut beds, mut scripts): (ResMut<BedRequests>, crate::ScriptHost),
) {
    for player in stops.0.drain(..) {
        if let Some((entity, ..)) = pcs.iter().find(|pc| pc.1.0 == player && pc.5.is_some()) {
            commands.entity(entity).remove::<(Resting, RestOffer)>();
            outbox.0.push((player, LedgerDelta::RestEnded));
        }
    }
    let in_world = session
        .players
        .values()
        .filter(|s| matches!(s, SessionPlayer::InWorld(_)))
        .count();
    let unconscious =
        |sheet: &Sheet| crate::is_unconscious(pc_status(&rules.0, &sheet.0).fatigue.current);
    // A bed (0x57b180 with a bed): no town map; its script, then whether it is taken.
    for (player, bed) in beds.0.drain(..) {
        let Some((entity, _, _, sheet, _, resting, _)) = pcs.iter().find(|pc| pc.1.0 == player)
        else {
            continue;
        };
        if resting.is_some() || in_world > 1 {
            continue;
        }
        if unconscious(sheet) {
            outbox.0.push((
                player,
                LedgerDelta::RestRefused {
                    message: ALREADY_UNCONSCIOUS,
                },
            ));
            continue;
        }
        if scripts.hook(
            &mut world,
            &mut map,
            &mut outbox,
            crate::ScriptObject::Map(bed),
            crate::EVENT_USE,
            player,
        ) == crate::HookResult::RunDefault
        {
            outbox.0.push((
                player,
                LedgerDelta::RestRefused {
                    message: CANNOT_SLEEP_HERE,
                },
            ));
            continue;
        }
        if pcs
            .iter()
            .any(|pc| pc.1.0 != player && pc.5.is_some_and(|r| r.bed == Some(bed)))
        {
            outbox.0.push((
                player,
                LedgerDelta::RestRefused {
                    message: BED_OCCUPIED,
                },
            ));
            continue;
        }
        commands.entity(entity).insert(RestOffer {
            mode: RestMode::Sleep,
            bed: Some(bed),
        });
        outbox.0.push((
            player,
            LedgerDelta::RestWindow {
                mode: RestMode::Sleep,
            },
        ));
    }
    for (player, option) in requests.0.drain(..) {
        let Some((entity, _, position, sheet, walking, resting, offer)) =
            pcs.iter().find(|pc| pc.1.0 == player)
        else {
            continue;
        };
        if resting.is_some() || in_world > 1 {
            continue;
        }
        if unconscious(sheet) {
            outbox.0.push((
                player,
                LedgerDelta::RestRefused {
                    message: ALREADY_UNCONSCIOUS,
                },
            ));
            continue;
        }
        // Resting in the bed the window was opened on.
        if let (
            Some(option),
            Some(&RestOffer {
                mode,
                bed: Some(bed),
            }),
        ) = (option, offer)
        {
            commands
                .entity(entity)
                .insert(Resting {
                    bed: Some(bed),
                    ..Resting::new(mode, option)
                })
                .remove::<(
                    RestOffer,
                    crate::Walking,
                    crate::PendingUse,
                    crate::PendingPick,
                    crate::PendingPickUp,
                )>();
            if walking {
                crate::record_position(&mut world, player, position.0);
            }
            continue;
        }
        let waiting = towns.0.get_or_insert_with(|| {
            vfs.as_ref()
                .map(|v| load_waiting_towns(&v.0.read().expect("vfs lock")))
                .unwrap_or_default()
        });
        let townmap = map
            .sectors
            .get(&position.0.sector())
            .map_or(0, |s| s.townmap_info);
        let mode = match rest_mode(townmap, waiting) {
            Ok(mode) => mode,
            Err(message) => {
                outbox
                    .0
                    .push((player, LedgerDelta::RestRefused { message }));
                continue;
            }
        };
        let Some(option) = option else {
            commands
                .entity(entity)
                .insert(RestOffer { mode, bed: None });
            outbox.0.push((player, LedgerDelta::RestWindow { mode }));
            continue;
        };
        if mode == RestMode::Wait && option == RestOption::UntilHealed {
            continue;
        }
        commands
            .entity(entity)
            .insert(Resting::new(mode, option))
            .remove::<(
                RestOffer,
                crate::Walking,
                crate::PendingUse,
                crate::PendingPick,
                crate::PendingPickUp,
            )>();
        if walking {
            crate::record_position(&mut world, player, position.0);
        }
    }
}

/// Steps the resting PCs an hour at a time: see the module docs.
#[allow(clippy::too_many_arguments)]
pub fn rest_steps(
    mut commands: Commands,
    mut clock: ResMut<crate::GameClock>,
    mut world: ResMut<WorldState>,
    session: Res<Session>,
    rules: Res<SimRules>,
    mut outbox: ResMut<Outbox>,
    mut pcs: Query<(
        Entity,
        &crate::NetId,
        &PlayerControlled,
        &mut Sheet,
        &mut Resting,
    )>,
) {
    for (entity, &net_id, player, mut sheet, mut resting) in &mut pcs {
        resting.ticks = resting.ticks.saturating_sub(1);
        if resting.ticks > 0 {
            continue;
        }
        clock.jump(u64::from(crate::HOUR_MS));
        if resting.mode == RestMode::Sleep {
            let heal_rate = rules.0.derived(&sheet.0).heal_rate;
            let times = if resting.bed.is_some() { 2 } else { 1 };
            let (hp, fatigue) =
                heal_at_rest(sheet.0.hp_damage, sheet.0.fatigue_damage, heal_rate, times);
            if (hp, fatigue) != (sheet.0.hp_damage, sheet.0.fatigue_damage) {
                (sheet.0.hp_damage, sheet.0.fatigue_damage) = (hp, fatigue);
                if let Some(saved) = world.0.as_mut().and_then(|w| w.players.get_mut(&player.0)) {
                    saved.blueprint.sheet = sheet.0.clone();
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
        if resting.go_on(clock.now.hour(), sheet.0.hp_damage) {
            resting.ticks = STEP_TICKS;
        } else {
            commands.entity(entity).remove::<Resting>();
            outbox.0.push((player.0, LedgerDelta::RestEnded));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_hour_at_rest_heals_hit_points_by_the_rate_and_fatigue_by_three_times_it() {
        assert_eq!(heal_at_rest(10, 20, 3, 1), (7, 11));
        assert_eq!(heal_at_rest(10, 20, 3, 2), (4, 2), "a bed doubles it");
        assert_eq!(heal_at_rest(2, 5, 3, 1), (0, 0), "not below 0");
        assert_eq!(heal_at_rest(0, -1, 3, 1), (0, -1), "nothing to heal");
    }

    #[test]
    fn each_option_takes_at_least_an_hour() {
        let mut four = Resting::new(RestMode::Sleep, RestOption::Hours(4));
        assert_eq!(
            (0..4).map(|_| four.go_on(12, 0)).collect::<Vec<_>>(),
            [true, true, true, false]
        );
        let mut morning = Resting::new(RestMode::Sleep, RestOption::UntilMorning);
        assert!(morning.go_on(6, 0) && !morning.go_on(MORNING, 0));
        let mut evening = Resting::new(RestMode::Wait, RestOption::UntilEvening);
        assert!(evening.go_on(7, 0) && !evening.go_on(EVENING, 0));
        let mut healed = Resting::new(RestMode::Sleep, RestOption::UntilHealed);
        assert!(healed.go_on(3, 1) && !healed.go_on(4, 0), "hit points only");
    }

    #[test]
    fn the_wilderness_is_for_sleeping_and_some_towns_for_waiting() {
        let mes = MesFile::parse(b"{0}{<none>}\n{1}{Town-Map Empty}\n{3}{Shrouded Hills [w:1]}\n{13}{The Lair of Bellerogrim 2}\n{14}{Tarant [W:1]}").unwrap();
        let waiting = waiting_towns(&mes);
        assert_eq!(waiting, BTreeSet::from([3, 14]));
        assert_eq!(rest_mode(0, &waiting), Ok(RestMode::Sleep));
        assert_eq!(rest_mode(3, &waiting), Ok(RestMode::Wait));
        assert_eq!(rest_mode(13, &waiting), Err(CANNOT_WAIT));
        assert_eq!(RestMode::Sleep.options().len(), 8);
        assert_eq!(RestMode::Wait.options().len(), 7);
        assert_eq!((RestMode::Sleep.label(7), RestMode::Wait.label(0)), (7, 12));
    }
}
