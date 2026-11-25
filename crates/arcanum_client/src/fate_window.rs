//! The fate points window (Arcanum.exe 0x56fae0 / 0x56fc70 / 0x56fe70; the host's side is
//! [`arcanum_sim::spend_fate`]).
//!
//! - **Opening:** the F key (0x54c67b) or the top bar's fate button (`lilgrnbut.art`, art 137, at
//!   (157, 9), hover help `intgame.mes` 1008 "Fate Points") opens or closes it (0x56fbf0).
//! - **The window:** `Fate_Base.ART` (art 292, 250x237) at (0, 41), under the top bar. Twelve
//!   rows, one per option of `mes/fate_ui.mes`: the text in Flare12Font at (11, 3 + 18 i), and a
//!   `FateBut.ART` button (art 293, 18x13) at (223, 4 + 18 i), shown pressed while that use
//!   waits (field 0xfe).
//! - **A click** spends a fate point on the option, or takes a waiting use back with its point
//!   (0x4f50c0 / 0x4f51b0): the host decides. The window stays open.
//! - **The count** of fate points is in the top bar, `%02d` at (190, 17, 24, 12) (0x551160 →
//!   0x54ad00 slot 2), from the PC's sheet ([`crate::hud`]).
//!
//! Builder's calls: Escape closes the window; nothing greys a row (as in the original).

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, ActivateOnPress, Button};

use arcanum_sim::PlayerIntent;

use crate::fonts::{ArcanumFonts, UiFont};
use crate::main_menu::AppState;
use crate::menu_window::{SCREEN_WHITE, at, px, rect};
use crate::pc_sheet::SheetHost;
use crate::resource_pack::paths::interface;
use crate::sim_bridge::HostLink;
use crate::widgets::FrameButton;

/// The window, in screen pixels.
pub const WINDOW: Rect = Rect {
    min: Vec2::new(0.0, 41.0),
    max: Vec2::new(250.0, 278.0),
};
/// Rows are this far apart.
pub const ROW_HEIGHT: f32 = 18.0;
/// The options.
pub const OPTIONS: usize = arcanum_sim::FATE_OPTIONS;

/// Whether the window is open.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FatePanel {
    pub open: bool,
}

impl FatePanel {
    /// Whether the open window covers `point` (screen pixels): clicks there are not the world's.
    pub fn covers(&self, point: Vec2) -> bool {
        self.open && WINDOW.contains(point)
    }
}

#[derive(Component)]
struct FateWindowRoot;

/// A row's button: the option.
#[derive(Component, Debug, Clone, Copy)]
struct FateRow(u8);

pub struct FateWindowPlugin;

impl Plugin for FateWindowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FatePanel>()
            .add_systems(OnExit(AppState::InGame), |mut panel: ResMut<FatePanel>| {
                *panel = FatePanel::default()
            })
            .add_systems(
                PreUpdate,
                toggle_on_keys
                    .run_if(in_state(AppState::InGame))
                    .after(bevy::input::InputSystems),
            )
            .add_systems(Update, show_window.run_if(in_state(AppState::InGame)))
            .add_observer(on_hud_button)
            .add_observer(on_row);
    }
}

/// F toggles the window; Escape closes it.
fn toggle_on_keys(mut keys: ResMut<ButtonInput<KeyCode>>, mut panel: ResMut<FatePanel>) {
    if keys.just_pressed(KeyCode::KeyF) {
        panel.open = !panel.open;
    } else if panel.open && keys.clear_just_pressed(KeyCode::Escape) {
        panel.open = false;
    }
}

fn on_hud_button(
    activate: On<Activate>,
    buttons: Query<&crate::hud::HudAction>,
    mut panel: ResMut<FatePanel>,
) {
    if buttons.get(activate.entity) == Ok(&crate::hud::HudAction::Fate) {
        panel.open = !panel.open;
    }
}

/// The waiting uses of the PC's sheet.
fn pending(sheet: &SheetHost) -> u16 {
    sheet.sheet().map_or(0, |s| s.fate_pending)
}

/// A row's button spends a point on its option, or takes a waiting one back.
fn on_row(
    activate: On<Activate>,
    rows: Query<&FateRow>,
    sheet: Res<SheetHost>,
    link: Option<Res<HostLink>>,
) {
    let (Ok(&FateRow(option)), Some(link)) = (rows.get(activate.entity), link) else {
        return;
    };
    let waiting = pending(&sheet) & arcanum_sim::fate_bit(option) != 0;
    let _ = link.0.send_intent(PlayerIntent::Fate {
        option,
        on: !waiting,
    });
}

fn art(name: &str) -> String {
    format!("{}/{name}", interface::DIR)
}

/// Spawns the window when it opens or what waits changes, and removes it when it closes.
#[allow(clippy::too_many_arguments)]
fn show_window(
    mut commands: Commands,
    panel: Res<FatePanel>,
    sheet: Res<SheetHost>,
    roots: Query<Entity, With<FateWindowRoot>>,
    assets: Res<AssetServer>,
    data: Option<Res<crate::create_character::CreationData>>,
    fonts: Option<Res<ArcanumFonts>>,
    mut shown: Local<Option<u16>>,
) {
    let want = panel.open.then(|| pending(&sheet));
    if *shown == want && (want.is_none() || !roots.is_empty()) {
        return;
    }
    *shown = want;
    for root in &roots {
        commands.entity(root).despawn();
    }
    let Some(waiting) = want else { return };
    let fonts = fonts.map(|f| f.clone()).unwrap_or_default();
    let local = |r: Rect| Rect {
        min: r.min + WINDOW.min,
        max: r.max + WINDOW.min,
    };
    commands
        .spawn((
            FateWindowRoot,
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
                ImageNode::new(assets.load(art("Fate_Base.ART"))),
                at(WINDOW),
            ));
            let frames = assets.load(art("FateBut.ART"));
            for row in 0..OPTIONS {
                let y = row as f32 * ROW_HEIGHT;
                let text = data
                    .as_ref()
                    .and_then(|d| d.fate_ui.get(row as u32))
                    .unwrap_or("")
                    .to_string();
                screen
                    .spawn((
                        bevy::picking::Pickable::IGNORE,
                        Node {
                            align_items: AlignItems::Center,
                            overflow: Overflow::clip(),
                            ..at(local(rect(
                                Vec2::new(11.0, 3.0 + y),
                                Vec2::new(WINDOW.width() - 11.0, ROW_HEIGHT),
                            )))
                        },
                    ))
                    .with_children(|cell| {
                        let mut t = cell.spawn(TextColor(SCREEN_WHITE));
                        fonts.insert_text(&mut t, &text, UiFont::Body);
                    });
                let held = waiting & arcanum_sim::fate_bit(row as u8) != 0;
                let frame = usize::from(held);
                screen.spawn((
                    FateRow(row as u8),
                    Button,
                    ActivateOnPress,
                    Hovered::default(),
                    FrameButton {
                        frames: frames.clone(),
                        normal: frame,
                        pressed: 1,
                        hover: frame,
                    },
                    ImageNode::default(),
                    at(local(rect(
                        Vec2::new(223.0, 4.0 + y),
                        Vec2::new(18.0, 13.0),
                    ))),
                ));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_sits_under_the_top_bar() {
        let panel = FatePanel { open: true };
        assert!(
            panel.covers(Vec2::new(10.0, 50.0))
                && !panel.covers(Vec2::new(10.0, 30.0))
                && !panel.covers(Vec2::new(260.0, 50.0))
        );
        assert!(!FatePanel::default().covers(Vec2::new(10.0, 50.0)));
    }
}
