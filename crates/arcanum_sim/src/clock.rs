//! The game clock, owned by the host (the original's clock module: tick 0x45b370, accessors
//! 0x45aaa0..0x45ab40).
//!
//! Game time is days and milliseconds of the day ([`GameTime`]), on a calendar of 12 months of
//! 30 days counted from 1 January of the session's first year. It runs at [`GAME_SPEED`] times
//! real time (`dt * 8`, 0x45b370: a game day is three real hours). A new game starts on day 0
//! at the hour `rules/gameinit.mes` gives (entry 5; 15:00 for Arcanum), in the year it gives
//! (entry 4; 1885).
//!
//! The host advances the clock every tick, saves it, and sends it in every state sync, so a
//! client knows the hour. The original stops the clock in single player while a menu is open
//! and in turn-based combat; neither exists here yet, and in multiplayer it never stops.

use arcanum_formats::mes::MesFile;
use arcanum_vfs::Vfs;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::TICK_HZ;

/// Game milliseconds per real millisecond.
pub const GAME_SPEED: u32 = 8;
/// Milliseconds in a day.
pub const DAY_MS: u32 = 86_400_000;
/// Milliseconds in an hour.
pub const HOUR_MS: u32 = 3_600_000;
/// The game rules file with the start year and hour.
pub const GAME_INIT: &str = "rules/gameinit.mes";

/// A moment of game time: whole days since the start of the first year, and milliseconds into
/// the day (always below [`DAY_MS`]).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct GameTime {
    pub days: u32,
    pub ms: u32,
}

impl GameTime {
    /// Day 0 at `hour` o'clock.
    pub fn at_hour(hour: u32) -> Self {
        Self {
            days: 0,
            ms: (hour % 24) * HOUR_MS,
        }
    }

    /// `ms` later (0x45aa10).
    pub fn plus_ms(self, ms: u64) -> Self {
        let total = u64::from(self.ms) + ms;
        Self {
            days: self.days + (total / u64::from(DAY_MS)) as u32,
            ms: (total % u64::from(DAY_MS)) as u32,
        }
    }

    /// 0 to 23 (0x45a870).
    pub fn hour(self) -> u32 {
        self.ms / HOUR_MS % 24
    }

    /// 0 to 59 (0x45a8b0).
    pub fn minute(self) -> u32 {
        self.ms / 60_000 % 60
    }

    /// 1 to 12 (0x45a920).
    pub fn month(self) -> u32 {
        self.days / 30 % 12 + 1
    }

    /// 1 to 30 (0x45a8e0).
    pub fn day_of_month(self) -> u32 {
        self.days % 30 + 1
    }

    /// The year, counting from `first_year` (0x45a900).
    pub fn year(self, first_year: u32) -> u32 {
        first_year + self.days / 360
    }

    /// Whether it is day for lighting and most of the game (0x45acf0): 6:00 to 17:59.
    pub fn is_day(self) -> bool {
        (6..18).contains(&self.hour())
    }
}

/// Where a new game's clock starts (`rules/gameinit.mes`, read by 0x4b9c30).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameStart {
    pub year: u32,
    pub hour: u32,
}

impl Default for GameStart {
    /// The original's fallback when the file is missing: 1885, 15:00.
    fn default() -> Self {
        Self {
            year: 1885,
            hour: 15,
        }
    }
}

impl GameStart {
    /// Reads entries 4 (year) and 5 (hour) of [`GAME_INIT`]; the defaults where they are missing.
    pub fn load(vfs: &Vfs) -> Self {
        let mut start = Self::default();
        let Some(mes) = vfs
            .read(GAME_INIT)
            .ok()
            .and_then(|d| MesFile::parse(&d).ok())
        else {
            return start;
        };
        let number = |key: u32| {
            mes.messages
                .get(&key)
                .and_then(|t| t.trim().parse::<u32>().ok())
        };
        if let Some(year) = number(4) {
            start.year = year;
        }
        if let Some(hour) = number(5) {
            start.hour = hour % 24;
        }
        start
    }
}

/// The host's clock: the time, and the part of a game millisecond carried between ticks.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GameClock {
    pub now: GameTime,
    /// Game milliseconds times [`TICK_HZ`], not yet added.
    carry: u32,
}

impl GameClock {
    pub fn at(now: GameTime) -> Self {
        Self { now, carry: 0 }
    }

    /// Jumps `ms` ahead at once (0x45c200: resting).
    pub fn jump(&mut self, ms: u64) {
        self.now = self.now.plus_ms(ms);
    }

    /// Advances by one host tick of real time.
    pub fn tick(&mut self) {
        // One tick is 1000 / TICK_HZ real ms: GAME_SPEED * 1000 / TICK_HZ game ms, kept exact.
        let hz = TICK_HZ as u32;
        self.carry += GAME_SPEED * 1000;
        let whole = self.carry / hz;
        self.carry %= hz;
        self.now = self.now.plus_ms(u64::from(whole));
    }
}

/// Advances the clock by one tick once a world exists, and has day and night locks follow it.
pub fn advance_clock(
    mut clock: ResMut<GameClock>,
    world: Res<crate::WorldState>,
    mut map: ResMut<crate::LoadedMap>,
) {
    if world.0.is_none() {
        return;
    }
    clock.tick();
    map.set_lock_day(crate::lock_day(clock.now.hour()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_calendar_has_twelve_months_of_thirty_days() {
        let t = GameTime {
            days: 359 + 31,
            ms: 15 * HOUR_MS + 30 * 60_000,
        };
        assert_eq!(
            (
                t.year(1885),
                t.month(),
                t.day_of_month(),
                t.hour(),
                t.minute()
            ),
            (1886, 2, 1, 15, 30)
        );
        assert_eq!(
            GameTime::at_hour(15).plus_ms(u64::from(9 * HOUR_MS)),
            GameTime { days: 1, ms: 0 },
            "midnight rolls the day"
        );
        assert!(GameTime::at_hour(6).is_day() && !GameTime::at_hour(18).is_day());
    }

    #[test]
    fn a_real_second_is_eight_game_seconds() {
        let mut clock = GameClock::at(GameTime::at_hour(15));
        for _ in 0..TICK_HZ as usize {
            clock.tick();
        }
        assert_eq!(
            clock.now,
            GameTime {
                days: 0,
                ms: 15 * HOUR_MS + 8000
            }
        );
        // A game day in three real hours.
        let mut clock = GameClock::default();
        for _ in 0..(3 * 3600 * TICK_HZ as usize) {
            clock.tick();
        }
        assert_eq!(clock.now, GameTime { days: 1, ms: 0 });
    }
}
