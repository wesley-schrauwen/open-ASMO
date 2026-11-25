//! The newspaper queue (Arcanum.exe module "Newspaper": init 0x4bf060, reset 0x4bf080, save /
//! load 0x4bf0c0 / 0x4bf100): which paper is out today, and the ones waiting.
//!
//! - **The queue** is 25 numbers: four past papers (-1 when none), today's, then up to twenty
//!   waiting. A new game starts with paper 5000 out (0x4bf080).
//! - **Queueing** (action 0x55, 0x4bf140): a paper below 5000 already out or waiting is not
//!   queued again; with a priority it goes first in line, else last (a full queue loses its last).
//! - **Each day** (0x4bf270 / 0x4bf2c0, a timed event every 24 hours; here when the game day
//!   changes): the next paper comes out. A scripted paper (below 5000) moves into the past; a
//!   stock one (5000 and up) is just replaced. With none waiting, a stock paper is picked at
//!   random: `(rand(0, m) + 500) × 10`, `m` the stock papers after 5000 in
//!   `mes/gamenewspaper.mes` (keys 5010, 5020, … to the first missing; 0x4bf330).
//! - **The headline** (action 0x56): today's paper's first entry, without its font and
//!   justification, ending in a full stop unless it already ends in `.`, `?` or `!`.
//!
//! Papers are `mes/gamenewspaper.mes` keys: the headline at the number, the rest after it.

use serde::{Deserialize, Serialize};

/// The texts.
pub const NEWSPAPER_MES: &str = "mes/gamenewspaper.mes";
/// Numbers from here on are stock papers.
pub const STOCK_PAPERS: i32 = 5000;
/// Where today's paper sits.
pub const TODAY: usize = 4;
/// The queue's length.
pub const QUEUE_LENGTH: usize = 25;

/// The queue, saved with the world's script state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Newspapers {
    pub queue: Vec<i32>,
    /// The game day the last paper came out.
    #[serde(default)]
    pub day: u32,
}

impl Default for Newspapers {
    /// A new game (0x4bf080).
    fn default() -> Self {
        Self { queue: vec![-1, -1, -1, -1, STOCK_PAPERS], day: 0 }
    }
}

impl Newspapers {
    /// Today's paper.
    pub fn today(&self) -> i32 {
        self.queue.get(TODAY).copied().unwrap_or(STOCK_PAPERS)
    }

    /// Queues paper `number` (0x4bf140).
    pub fn queue(&mut self, number: i32, priority: bool) {
        if number < STOCK_PAPERS && self.queue.iter().skip(TODAY).any(|&n| n == number) {
            return;
        }
        if priority {
            self.queue.insert(TODAY + 1, number);
            self.queue.truncate(QUEUE_LENGTH);
        } else if self.queue.len() < QUEUE_LENGTH {
            self.queue.push(number);
        } else {
            self.queue[QUEUE_LENGTH - 1] = number;
        }
    }

    /// The next day's paper comes out (0x4bf2c0); `stock` picks a stock paper when none waits.
    pub fn next_day(&mut self, stock: impl FnOnce() -> i32) {
        let start = if self.today() < STOCK_PAPERS { 0 } else { TODAY };
        if start < self.queue.len() {
            self.queue.remove(start);
        }
        if self.queue.len() <= TODAY {
            self.queue.resize(TODAY, -1);
            self.queue.push(stock());
        }
    }
}

/// A random stock paper (0x4bf330): `m` stock papers after the first.
pub fn stock_paper(m: i32, roll: i32) -> i32 {
    (roll.clamp(0, m.max(0)) + 500) * 10
}

/// How many stock papers follow 5000 in `mes` (5010, 5020, … to the first missing).
pub fn stock_count(mes: &arcanum_formats::mes::MesFile) -> i32 {
    (1..).take_while(|k| mes.get((STOCK_PAPERS + 10 * k) as u32).is_some()).count() as i32
}

/// A paper's headline as action 0x56 floats it.
pub fn headline(entry: &str) -> String {
    let text = entry.trim_start().trim_start_matches(|c: char| c.is_ascii_digit()).trim_start();
    let text = text.get(1..).unwrap_or("").trim().replace('\n', " ");
    if text.ends_with(['.', '?', '!']) { text } else { format!("{text}.") }
}

/// A new paper for each game day passed (at most a queue's worth at once).
pub fn advance_newspapers(mut world: bevy_ecs::prelude::ResMut<crate::WorldState>, clock: bevy_ecs::prelude::Res<crate::GameClock>, vfs: Option<bevy_ecs::prelude::Res<crate::SimVfs>>, mut rng: bevy_ecs::prelude::ResMut<crate::HostRng>, mut stock: bevy_ecs::prelude::Local<Option<i32>>) {
    let today = clock.now.days;
    let behind = world.0.as_ref().map_or(0, |s| today.saturating_sub(s.scripts.newspapers.day));
    if behind == 0 {
        return;
    }
    let m = *stock.get_or_insert_with(|| {
        let guard = vfs.as_ref().and_then(|v| v.0.read().ok());
        guard.as_ref().and_then(|v| v.read(NEWSPAPER_MES).ok()).and_then(|d| arcanum_formats::mes::MesFile::parse(&d).ok()).map_or(0, |m| stock_count(&m))
    });
    let Some(save) = world.0.as_mut() else { return };
    let papers = &mut save.scripts.newspapers;
    for _ in 0..behind.min(QUEUE_LENGTH as u32) {
        let roll = rng.range(0, m.max(0));
        papers.next_day(|| stock_paper(m, roll));
    }
    papers.day = today;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn papers_queue_come_out_daily_and_stock_fills_in() {
        let mut papers = Newspapers::default();
        assert_eq!(papers.today(), 5000);
        papers.queue(1020, false);
        papers.queue(1030, false);
        papers.queue(1020, true);
        assert_eq!(papers.queue, [-1, -1, -1, -1, 5000, 1020, 1030], "not twice");
        papers.queue(1040, true);
        assert_eq!(&papers.queue[5..], [1040, 1020, 1030], "first in line");
        papers.next_day(|| 5010);
        assert_eq!((papers.today(), papers.queue[..4].to_vec()), (1040, vec![-1, -1, -1, -1]), "stock replaced");
        papers.next_day(|| 5010);
        assert_eq!((papers.today(), papers.queue[3]), (1020, 1040), "a scripted paper goes into the past");
        papers.next_day(|| 5010);
        papers.next_day(|| 5020);
        assert_eq!(papers.today(), 5020, "none waiting: stock");
        assert_eq!(papers.queue.len(), 5);
        assert_eq!(stock_paper(16, 3), 5030);
    }

    #[test]
    fn headlines_lose_their_font_and_end_in_a_stop() {
        assert_eq!(headline("490c TECHNOLOGY PROTEST\nCLAIMS 3 LIVES"), "TECHNOLOGY PROTEST CLAIMS 3 LIVES.");
        assert_eq!(headline("490c SAMPLE HEADLINE!"), "SAMPLE HEADLINE!");
    }
}
