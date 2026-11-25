//! The big-move vote (owner's rules, [`arcanum_sim::big_move`]): a teleport waits until every
//! player has said yes; one no cancels it. The original has no such window (it skips these
//! moves in multiplayer), so its look is the builder's: a black panel like the dialogue's
//! options, a title ("Confirm Group Action", owner's wording), one line naming who wants the move, then "Yes" and "No" rows (white, cyan under the
//! cursor). Several votes queue; the oldest shows. A vote that ends (either way) goes.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, ActivateOnPress, Button};

use arcanum_sim::big_move::VoteAsk;
use arcanum_sim::{LedgerDelta, PlayerIntent};

use crate::fonts::{ArcanumFonts, UiFont};
use crate::main_menu::AppState;
use crate::menu_window::{at, px, rect};
use crate::sim_bridge::HostLink;

/// The panel, in screen pixels (above the dialogue options' place).
pub const PANEL: Rect = Rect { min: Vec2::new(250.0, 200.0), max: Vec2::new(550.0, 301.0) };
/// The panel's title (owner's wording).
pub const TITLE: &str = "Confirm Group Action";
const ROW_HEIGHT: f32 = 19.0;
const WHITE: Color = Color::WHITE;
const CYAN: Color = Color::srgb(0.0, 1.0, 1.0);

/// The votes waiting for this player's answer, oldest first.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct Votes {
    pub open: Vec<VoteAsk>,
    /// Answered here, waiting for the host to end them.
    pub answered: Vec<u32>,
    changed: bool,
}

impl Votes {
    pub fn receive(&mut self, delta: &LedgerDelta) {
        match delta {
            LedgerDelta::MoveVote { ask } => {
                self.open.push(ask.clone());
                self.changed = true;
            }
            LedgerDelta::VoteEnded { id, .. } => {
                self.open.retain(|v| v.id != *id);
                self.answered.retain(|a| a != id);
                self.changed = true;
            }
            _ => {}
        }
    }

    /// The vote to show: the oldest not answered yet.
    pub fn shown(&self) -> Option<&VoteAsk> {
        self.open.iter().find(|v| !self.answered.contains(&v.id))
    }

    /// The line naming who wants the move (builder's wording; the original has none).
    pub fn line(ask: &VoteAsk) -> String {
        format!("{} wants to go elsewhere. Go along?", ask.causer)
    }
}

#[derive(Component)]
struct VotePanel;

/// A row: yes or no.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct VoteRow(bool);

pub struct VotePopupPlugin;

impl Plugin for VotePopupPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Votes>()
            .add_systems(OnExit(AppState::InGame), |mut votes: ResMut<Votes>| *votes = Votes::default())
            .add_systems(Update, (show_panel, colour_rows).chain().run_if(in_state(AppState::InGame)))
            .add_observer(on_row);
    }
}

fn show_panel(mut commands: Commands, mut votes: ResMut<Votes>, panels: Query<Entity, With<VotePanel>>, fonts: Option<Res<ArcanumFonts>>) {
    if !votes.changed {
        return;
    }
    votes.changed = false;
    for panel in &panels {
        commands.entity(panel).despawn();
    }
    let Some(ask) = votes.shown().cloned() else { return };
    let fonts = fonts.map(|f| f.clone()).unwrap_or_default();
    commands
        .spawn((
            VotePanel,
            DespawnOnExit(AppState::InGame),
            bevy::picking::Pickable::IGNORE,
            Node { position_type: PositionType::Absolute, top: Val::Px(0.0), left: Val::Percent(50.0), margin: UiRect::left(px(-400.0)), width: px(800.0), height: px(600.0), ..default() },
        ))
        .with_children(|screen| {
            screen.spawn((BackgroundColor(Color::BLACK), at(PANEL)));
            let title = rect(PANEL.min + Vec2::new(6.0, 3.0), Vec2::new(PANEL.width() - 12.0, ROW_HEIGHT));
            screen.spawn((bevy::picking::Pickable::IGNORE, Node { overflow: Overflow::clip(), ..at(title) })).with_children(|cell| {
                let mut t = cell.spawn((TextColor(WHITE), bevy::picking::Pickable::IGNORE));
                fonts.insert_text(&mut t, TITLE, UiFont::Small);
            });
            let line = rect(PANEL.min + Vec2::new(6.0, 3.0 + ROW_HEIGHT), Vec2::new(PANEL.width() - 12.0, ROW_HEIGHT * 2.0));
            screen.spawn((bevy::picking::Pickable::IGNORE, Node { overflow: Overflow::clip(), ..at(line) })).with_children(|cell| {
                let mut t = cell.spawn((TextColor(WHITE), bevy::picking::Pickable::IGNORE));
                fonts.insert_text(&mut t, &Votes::line(&ask), UiFont::Body);
            });
            for (i, (yes, label)) in [(true, "Yes"), (false, "No")].into_iter().enumerate() {
                let row = rect(PANEL.min + Vec2::new(6.0, 3.0 + ROW_HEIGHT * (3 + i) as f32), Vec2::new(PANEL.width() - 12.0, ROW_HEIGHT));
                screen.spawn((VoteRow(yes), Button, ActivateOnPress, Hovered::default(), Node { overflow: Overflow::clip(), ..at(row) })).with_children(|cell| {
                    let mut t = cell.spawn((TextColor(WHITE), bevy::picking::Pickable::IGNORE));
                    fonts.insert_text(&mut t, label, UiFont::Body);
                });
            }
        });
}

fn colour_rows(rows: Query<(&Hovered, &Children), With<VoteRow>>, mut colours: Query<&mut TextColor>) {
    for (hovered, children) in &rows {
        for &child in children {
            if let Ok(mut c) = colours.get_mut(child) {
                c.set_if_neq(TextColor(if hovered.0 { CYAN } else { WHITE }));
            }
        }
    }
}

fn on_row(activate: On<Activate>, rows: Query<&VoteRow>, mut votes: ResMut<Votes>, link: Option<Res<HostLink>>) {
    let Ok(&VoteRow(yes)) = rows.get(activate.entity) else { return };
    let Some(id) = votes.shown().map(|v| v.id) else { return };
    if let Some(link) = link
        && let Err(e) = link.0.send_intent(PlayerIntent::Vote { id, yes })
    {
        warn!("cannot reach the host: {e}");
    }
    votes.answered.push(id);
    votes.changed = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_unanswered_vote_shows_until_the_host_ends_it() {
        let mut votes = Votes::default();
        let ask = |id| VoteAsk { id, causer: "Virgil".into() };
        votes.receive(&LedgerDelta::MoveVote { ask: ask(1) });
        votes.receive(&LedgerDelta::MoveVote { ask: ask(2) });
        assert_eq!(votes.shown().map(|v| v.id), Some(1));
        votes.answered.push(1);
        assert_eq!(votes.shown().map(|v| v.id), Some(2), "answered: the next one");
        votes.receive(&LedgerDelta::VoteEnded { id: 1, passed: true });
        votes.receive(&LedgerDelta::VoteEnded { id: 2, passed: false });
        assert!(votes.shown().is_none() && votes.open.is_empty() && votes.answered.is_empty());
        assert_eq!(Votes::line(&ask(3)), "Virgil wants to go elsewhere. Go along?");
    }
}
