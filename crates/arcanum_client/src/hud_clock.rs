//! The clock in the in-game top bar (Arcanum.exe `intgame_clock_refresh`, 0x5522f0).
//!
//! - A 512-pixel ring scrolls through the clock's 128x30 window at (648, 5): the day strip
//!   (`Clk_timestrip`, art 207, 384 wide: night, dawn, sun, dusk) followed by one moon picture
//!   (`Clk_Moon1`-`8`, art 208-215, 128 wide). Past the moon the ring starts over with the strip.
//! - With `S` the game time in whole seconds, the ring position at the window's centre is
//!   `((S + 73800) mod 86400) * 512 / 86400`, rounded down: the strip's left edge is at the
//!   centre at 3:30, the sun near noon, the moon near 0:30 ([`ring_offset`]).
//! - The moon picture is the phase of a 28-day cycle counted from day 0, changing at noon
//!   ([`moon_picture`], table 0x5c730c).
//! - A pointer (`TimePnts`, art 216, 5x29) is fixed at (708, 6), the window's centre.
//! - The original redraws it when the interface starts and then once a game hour (a timed event
//!   of 3,600,000 game ms, 0x5526f0): the ring moves about 21 pixels at a time.
//!
//! - Hovering it shows "Current Time: 15:00:00   Current Date: 1/1/1885" in the message box
//!   (0x54b5d0, kind 6, one line), the time and date taken once as the hover starts
//!   ([`clock_help`]).
//!
//! Not built: the "%d Hours" text branch (0x55260b), which nothing in the exe turns on.

use arcanum_sim::GameTime;
use bevy::prelude::*;

use crate::actors::WorldClock;
use crate::menu_window::{at, px, rect};
use crate::resource_pack::paths::interface;

/// The clock window in the top bar.
pub const CLOCK: Rect = Rect {
    min: Vec2::new(648.0, 5.0),
    max: Vec2::new(776.0, 35.0),
};
pub const STRIP_WIDTH: u32 = 384;
pub const MOON_WIDTH: u32 = 128;
const RING: u32 = STRIP_WIDTH + MOON_WIDTH;
/// Seconds added before taking the time of day (0x12048): puts 3:30 at the strip's left edge.
const RING_START: u64 = 73_800;
/// Last day of each moon picture's phases, out of a 28-day cycle (0x5c730c).
const MOON_PHASES: [u64; 8] = [0, 4, 9, 13, 14, 18, 23, 27];
/// Game milliseconds between redraws.
pub const REDRAW_MS: u64 = 3_600_000;

fn seconds(t: GameTime) -> u64 {
    u64::from(t.days) * 86_400 + u64::from(t.ms / 1000)
}

fn total_ms(t: GameTime) -> u64 {
    u64::from(t.days) * 86_400_000 + u64::from(t.ms)
}

/// How far into the ring the clock window's left edge is at `t`.
pub fn ring_offset(t: GameTime) -> u32 {
    let centre = ((seconds(t) + RING_START) % 86_400) * u64::from(RING) / 86_400;
    (centre as u32 + RING - CLOCK.width() as u32 / 2) % RING
}

/// Which moon picture (0 for `Clk_Moon1` to 7) shows at `t`.
pub fn moon_picture(t: GameTime) -> usize {
    let phase = (seconds(t) + 43_200) / 86_400 % 28;
    MOON_PHASES
        .iter()
        .position(|&last| phase <= last)
        .unwrap_or(7)
}

/// `%02d:%02d:%02d`, 24-hour (0x45ac20).
pub fn time_text(t: GameTime) -> String {
    format!("{:02}:{:02}:{:02}", t.hour(), t.minute(), t.ms / 1000 % 60)
}

/// `%d/%d/%d` (0x45ac70). The original orders it by the Windows locale (`LOCALE_IDATE`: month
/// first, day first or year first); this uses its fallback, month first.
pub fn date_text(t: GameTime, first_year: u32) -> String {
    format!("{}/{}/{}", t.month(), t.day_of_month(), t.year(first_year))
}

/// The clock's hover help: `"%s: %s"` then `"   %s: %s"` with `intgame.mes` 22 and 23.
pub fn clock_help(t: GameTime, first_year: u32, time_label: &str, date_label: &str) -> String {
    format!(
        "{time_label}: {}   {date_label}: {}",
        time_text(t),
        date_text(t, first_year)
    )
}

/// The ring's pictures inside the clock window, in the window's pixels.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingPart {
    Strip,
    Moon,
    /// The strip again after the moon, where the ring starts over.
    StripAgain,
}

impl RingPart {
    fn left(self, offset: u32) -> f32 {
        let start = match self {
            Self::Strip => 0,
            Self::Moon => STRIP_WIDTH,
            Self::StripAgain => RING,
        };
        start as f32 - offset as f32
    }
}

/// The game time the clock was last drawn for.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct ClockFace {
    drawn: Option<GameTime>,
}

/// The moon pictures, loaded with the clock.
#[derive(Component, Debug, Clone)]
pub struct Moons(Vec<Handle<Image>>);

/// Spawns the clock into the top bar.
pub fn spawn_clock(top: &mut ChildSpawnerCommands, assets: &AssetServer) {
    let strip: Handle<Image> = assets.load(interface::CLOCK_STRIP);
    let moons: Vec<Handle<Image>> = (1..=8)
        .map(|n| assets.load(format!("{}/Clk_Moon{n}.ART", interface::DIR)))
        .collect();
    let height = 29.0;
    top.spawn((
        ClockFace::default(),
        Moons(moons.clone()),
        BackgroundColor(Color::BLACK),
        Node {
            overflow: Overflow::clip(),
            ..at(CLOCK)
        },
    ))
    .with_children(|face| {
        for (part, image) in [
            (RingPart::Strip, strip.clone()),
            (RingPart::Moon, moons[0].clone()),
            (RingPart::StripAgain, strip),
        ] {
            let width = if part == RingPart::Moon {
                MOON_WIDTH
            } else {
                STRIP_WIDTH
            };
            face.spawn((
                part,
                ImageNode::new(image),
                at(rect(
                    Vec2::new(part.left(0), 0.0),
                    Vec2::new(width as f32, height),
                )),
            ));
        }
    });
    top.spawn(crate::message_box::help_area(
        crate::message_box::BoxHelp::Clock,
        CLOCK,
    ));
    top.spawn((
        ImageNode::new(assets.load(interface::CLOCK_POINTER)),
        at(rect(Vec2::new(708.0, 6.0), Vec2::new(5.0, 29.0))),
    ));
}

/// Redraws the clock for the world's time when it is first known and a game hour after each
/// redraw, as the original does.
pub fn show_clock(
    clock: Res<WorldClock>,
    mut faces: Query<(&mut ClockFace, &Moons, &Children)>,
    mut parts: Query<(&RingPart, &mut Node, &mut ImageNode)>,
) {
    let Some(now) = clock.now else { return };
    for (mut face, moons, children) in &mut faces {
        let due = match face.drawn {
            None => true,
            // A time from before the last drawing (a load or a host's correction) redraws too.
            Some(drawn) => now < drawn || total_ms(now) >= total_ms(drawn) + REDRAW_MS,
        };
        if !due {
            continue;
        }
        face.drawn = Some(now);
        let offset = ring_offset(now);
        let moon = &moons.0[moon_picture(now)];
        for &child in children {
            let Ok((&part, mut node, mut image)) = parts.get_mut(child) else {
                continue;
            };
            node.left = px(part.left(offset));
            if part == RingPart::Moon && image.image != *moon {
                image.image = moon.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_time(days: u32, hour: u32, minute: u32) -> GameTime {
        GameTime {
            days,
            ms: (hour * 60 + minute) * 60_000,
        }
    }

    /// The ring position at the window's centre.
    fn centre(t: GameTime) -> u32 {
        (ring_offset(t) + 64) % RING
    }

    #[test]
    fn the_clock_help_gives_the_time_and_date() {
        let t = GameTime {
            days: 365,
            ms: ((15 * 60 + 4) * 60 + 9) * 1000 + 999,
        };
        assert_eq!(
            clock_help(t, 1885, "Current Time", "Current Date"),
            "Current Time: 15:04:09   Current Date: 1/6/1886"
        );
        assert_eq!(date_text(GameTime::default(), 1885), "1/1/1885");
    }

    #[test]
    fn the_ring_turns_once_a_day() {
        assert_eq!(
            centre(at_time(0, 3, 30)),
            0,
            "the strip's left edge at 3:30"
        );
        assert_eq!(
            ring_offset(at_time(0, 3, 30)),
            448,
            "the moon's right half shows to its left"
        );
        assert_eq!(centre(at_time(0, 0, 30)), 448, "the moon's middle at 0:30");
        assert_eq!(centre(at_time(0, 12, 0)), 181, "the sun near noon");
        assert_eq!(centre(at_time(5, 12, 0)), 181, "the same every day");
        // 512 pixels a day: about 21 an hour.
        assert_eq!(centre(at_time(0, 13, 0)) - centre(at_time(0, 12, 0)), 21);
    }

    #[test]
    fn the_moon_follows_a_28_day_cycle_changing_at_noon() {
        assert_eq!(moon_picture(at_time(0, 11, 59)), 0);
        assert_eq!(moon_picture(at_time(0, 12, 0)), 1, "day 1 from noon");
        assert_eq!(moon_picture(at_time(4, 0, 0)), 1);
        assert_eq!(
            moon_picture(at_time(13, 23, 0)),
            4,
            "day 14: the full moon alone"
        );
        assert_eq!(moon_picture(at_time(27, 13, 0)), 0, "day 28: new again");
        assert_eq!(moon_picture(at_time(26, 13, 0)), 7);
    }

    #[test]
    fn ring_parts_sit_end_to_end() {
        assert_eq!(RingPart::Strip.left(300), -300.0);
        assert_eq!(
            RingPart::Moon.left(300),
            84.0,
            "the moon follows the strip's last 84 pixels"
        );
        assert_eq!(
            RingPart::StripAgain.left(450),
            62.0,
            "past the moon the strip starts over"
        );
    }
}
