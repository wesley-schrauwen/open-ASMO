//! The sleep window (Arcanum.exe's sleep interface, 0x57b080 / 0x57b180 / 0x57b4f0 / 0x57b710;
//! the host's side is [`arcanum_sim::rest`]).
//!
//! - **Opening:** the S key (0x54c6bf) or the top bar's sleep button (`lilgrnbut.art`, art 137,
//!   at (605, 9), hover help `intgame.mes` 1009) asks the host; it answers with the window's
//!   mode, or a refusal from `mes/SleepUI.mes` that shows in the message box. S or the button
//!   again closes it, and stops a rest under way (0x57b450).
//! - **The window:** `Sleep_Base.ART` (art 565, 227x152) at (573, 42). One row per option
//!   (8 sleeping, 7 waiting): a `FateBut.ART` button (art 293, 18x13) at (8, 3 + 18 k) and its
//!   text (`SleepUI.mes` k, or 12 + k when waiting) in Flare12Font at (36, 2 + 18 k, 191, 18).
//! - **Resting:** a click on a row's button asks the host to rest, and the screen fades to
//!   black (0x57b9e0) until the host says the rest is over; then the window closes (0x57ba70).
//!
//! Builder's calls: Escape closes the window when no rest is under way; the fade takes
//! [`FADE_SECS`] (the original's speed was not read) and comes back at once; the text is white.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, ActivateOnPress, Button};

use arcanum_sim::{LedgerDelta, PlayerIntent, RestMode};

use crate::fonts::{ArcanumFonts, UiFont};
use crate::main_menu::AppState;
use crate::menu_window::{SCREEN_WHITE, at, px, rect};
use crate::resource_pack::paths::interface;
use crate::sim_bridge::HostLink;
use crate::widgets::FrameButton;

/// The window, in screen pixels.
pub const WINDOW: Rect = Rect {
    min: Vec2::new(573.0, 42.0),
    max: Vec2::new(800.0, 194.0),
};
/// Rows are this far apart.
pub const ROW_HEIGHT: f32 = 18.0;
/// Seconds the screen takes to go black.
pub const FADE_SECS: f32 = 0.4;

/// The sleep window's state: open in a mode, and whether a rest is under way.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RestPanel {
    pub mode: Option<RestMode>,
    pub resting: bool,
}

impl RestPanel {
    /// Takes the host's answers.
    pub fn receive(&mut self, delta: &LedgerDelta) {
        match delta {
            LedgerDelta::RestWindow { mode } => {
                *self = Self {
                    mode: Some(*mode),
                    resting: false,
                }
            }
            LedgerDelta::RestEnded => *self = Self::default(),
            _ => {}
        }
    }

    /// Whether the open window covers `point` (screen pixels): clicks there are not the world's.
    pub fn covers(&self, point: Vec2) -> bool {
        self.mode.is_some() && WINDOW.contains(point)
    }
}

/// The window's root.
#[derive(Component)]
struct RestWindowRoot;

/// A row's button: the row number.
#[derive(Component, Debug, Clone, Copy)]
struct RestRow(usize);

/// The black over the screen while resting, and when it started.
#[derive(Component)]
struct RestFade(f32);

pub struct RestWindowPlugin;

impl Plugin for RestWindowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RestPanel>()
            .add_systems(OnExit(AppState::InGame), |mut panel: ResMut<RestPanel>| {
                *panel = RestPanel::default()
            })
            .add_systems(
                PreUpdate,
                toggle_on_keys
                    .run_if(in_state(AppState::InGame))
                    .after(bevy::input::InputSystems),
            )
            .add_systems(
                Update,
                (show_window, fade).run_if(in_state(AppState::InGame)),
            )
            .add_observer(on_hud_button)
            .add_observer(on_row);
    }
}

/// Opens the window by asking the host, or closes it (stopping a rest under way).
fn toggle(panel: &mut RestPanel, link: Option<&HostLink>) {
    let Some(link) = link else { return };
    if panel.mode.is_some() {
        if panel.resting {
            let _ = link.0.send_intent(PlayerIntent::StopRest);
        }
        *panel = RestPanel::default();
    } else {
        let _ = link.0.send_intent(PlayerIntent::RestWindow);
    }
}

/// S toggles the window; Escape closes it when no rest is under way.
fn toggle_on_keys(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut panel: ResMut<RestPanel>,
    link: Option<Res<HostLink>>,
) {
    if keys.just_pressed(KeyCode::KeyS) {
        toggle(&mut panel, link.as_deref());
    } else if panel.mode.is_some() && !panel.resting && keys.clear_just_pressed(KeyCode::Escape) {
        *panel = RestPanel::default();
    }
}

fn on_hud_button(
    activate: On<Activate>,
    buttons: Query<&crate::hud::HudAction>,
    mut panel: ResMut<RestPanel>,
    link: Option<Res<HostLink>>,
) {
    if buttons.get(activate.entity) == Ok(&crate::hud::HudAction::Sleep) {
        toggle(&mut panel, link.as_deref());
    }
}

/// A row's button rests for its option (0x57b710).
fn on_row(
    activate: On<Activate>,
    rows: Query<&RestRow>,
    mut panel: ResMut<RestPanel>,
    link: Option<Res<HostLink>>,
) {
    let (Ok(&RestRow(row)), Some(mode), Some(link)) = (rows.get(activate.entity), panel.mode, link)
    else {
        return;
    };
    if panel.resting {
        return;
    }
    let Some(&option) = mode.options().get(row) else {
        return;
    };
    if link.0.send_intent(PlayerIntent::Rest { option }).is_ok() {
        panel.resting = true;
    }
}

fn art(name: &str) -> String {
    format!("{}/{name}", interface::DIR)
}

/// Spawns the window when it opens and removes it when it closes.
fn show_window(
    mut commands: Commands,
    panel: Res<RestPanel>,
    roots: Query<Entity, With<RestWindowRoot>>,
    assets: Res<AssetServer>,
    data: Option<Res<crate::create_character::CreationData>>,
    fonts: Option<Res<ArcanumFonts>>,
    mut shown: Local<Option<RestMode>>,
) {
    if *shown == panel.mode && (panel.mode.is_none() || !roots.is_empty()) {
        return;
    }
    *shown = panel.mode;
    for root in &roots {
        commands.entity(root).despawn();
    }
    let Some(mode) = panel.mode else { return };
    let fonts = fonts.map(|f| f.clone()).unwrap_or_default();
    let local = |r: Rect| Rect {
        min: r.min + WINDOW.min,
        max: r.max + WINDOW.min,
    };
    commands
        .spawn((
            RestWindowRoot,
            DespawnOnExit(AppState::InGame),
            bevy::picking::Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Percent(50.0),
                margin: UiRect::left(px(-400.0)),
                width: px(800.0),
                height: px(600.0),
                ..default()
            },
        ))
        .with_children(|screen| {
            screen.spawn((
                ImageNode::new(assets.load(art("Sleep_Base.ART"))),
                at(WINDOW),
            ));
            let frames = assets.load(art("FateBut.ART"));
            for (row, _) in mode.options().iter().enumerate() {
                let y = row as f32 * ROW_HEIGHT;
                screen.spawn((
                    RestRow(row),
                    Button,
                    ActivateOnPress,
                    Hovered::default(),
                    FrameButton {
                        frames: frames.clone(),
                        normal: 0,
                        pressed: 1,
                        hover: 0,
                    },
                    ImageNode::default(),
                    at(local(rect(Vec2::new(8.0, 3.0 + y), Vec2::new(18.0, 13.0)))),
                ));
                let text = data
                    .as_ref()
                    .and_then(|d| d.sleep_ui.get(mode.label(row)))
                    .unwrap_or("")
                    .to_string();
                screen
                    .spawn((
                        bevy::picking::Pickable::IGNORE,
                        Node {
                            align_items: AlignItems::Center,
                            overflow: Overflow::clip(),
                            ..at(local(rect(
                                Vec2::new(36.0, 2.0 + y),
                                Vec2::new(191.0, ROW_HEIGHT),
                            )))
                        },
                    ))
                    .with_children(|cell| {
                        let mut t = cell.spawn(TextColor(SCREEN_WHITE));
                        fonts.insert_text(&mut t, &text, UiFont::Body);
                    });
            }
        });
}

/// Fades the screen to black while resting.
fn fade(
    mut commands: Commands,
    panel: Res<RestPanel>,
    time: Res<Time<Real>>,
    mut fades: Query<(Entity, &RestFade, &mut BackgroundColor)>,
) {
    let now = time.elapsed_secs();
    if !panel.resting {
        for (entity, ..) in &fades {
            commands.entity(entity).despawn();
        }
        return;
    }
    if fades.is_empty() {
        commands.spawn((
            RestFade(now),
            DespawnOnExit(AppState::InGame),
            GlobalZIndex(20),
            BackgroundColor(Color::NONE),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
        ));
        return;
    }
    for (_, &RestFade(start), mut colour) in &mut fades {
        let alpha = ((now - start) / FADE_SECS).clamp(0.0, 1.0);
        colour.0 = Color::BLACK.with_alpha(alpha);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_follows_the_host() {
        let mut panel = RestPanel::default();
        assert!(!panel.covers(Vec2::new(600.0, 100.0)), "closed");
        panel.receive(&LedgerDelta::RestWindow {
            mode: RestMode::Wait,
        });
        assert_eq!(
            panel,
            RestPanel {
                mode: Some(RestMode::Wait),
                resting: false
            }
        );
        assert!(panel.covers(Vec2::new(600.0, 100.0)));
        assert!(!panel.covers(Vec2::new(560.0, 100.0)) && !panel.covers(Vec2::new(600.0, 200.0)));
        panel.resting = true;
        panel.receive(&LedgerDelta::RestEnded);
        assert_eq!(panel, RestPanel::default());
    }
}
