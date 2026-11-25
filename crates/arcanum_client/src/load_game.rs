//! The Load Game screen, reached from the Single Player menu.
//!
//! Built from the original's art and its screen table in Arcanum.exe (0x5c4560): the 800x41 top
//! bar, the 800x400 save/load background and the 800x159 bottom bar, stacked into the 800x600
//! screen. The background has a save list on the left, a portrait socket above it, and an oval
//! window for the save's screenshot. Buttons, at the original's positions:
//! - Back (`BackBut`) and Load (`NextBut`) in the bottom bar's round sockets;
//! - Delete (`cncl_big`) in the small circle under the list;
//! - the info toggle (`TextToggle`) at the top of the oval.
//!
//! Hovering a button shows its hover help (`MainMenu.mes` 7002, 7020, 7022, 7025) in the
//! bottom bar's text box. The bars, Back/Next and hover help are shared with the other menu
//! screens ([`crate::menu_window`]).
//!
//! [`SaveGames`] is filled on entering the screen from the host's save folder
//! ([`HostSettings::save_dir`](crate::sim_bridge::HostSettings)), newest first: each save shows
//! the player's PC name and level, the map, and when it was written. Load starts a host from the
//! selected save ([`crate::sim_bridge::boot_save`]) and goes through the loading screen into the
//! world once the PC is back. Delete only removes the entry from the list: deleting the file
//! waits for the confirmation (`MainMenu.mes` 5150).

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, ActivateOnPress, Button};

use crate::art_loader::ArtFrames;
use crate::audio::MenuSounds;
use crate::fonts::{ArcanumFonts, UiFont};
use crate::loading::LoadRequest;
use crate::main_menu::{AppState, HOVER_COLOR};
pub use crate::menu_window::{HoverHelp, HoverHelpText, INFO_COLOR};
use crate::menu_window::{
    MenuWindowPlugin, TOP_BAR_HEIGHT, WindowArt, at, image_button, rect, screen_root, spawn_frame,
};
use crate::resource_pack::paths::interface;
use crate::widgets::HoverText;

/// A save game as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveSummary {
    /// The name the player gave the save (the PC's name until saves are named).
    pub name: String,
    /// The save file; `None` for the examples.
    pub path: Option<std::path::PathBuf>,
    pub level: u32,
    pub location: String,
    /// When it was saved, ready to display.
    pub saved_at: String,
}

/// The saves the Load Game screen lists, newest first.
#[derive(Resource, Debug, Clone, Default)]
pub struct SaveGames(pub Vec<SaveSummary>);

/// [`SaveGames`] holds examples: do not replace them from the save folder (screenshots).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct ExampleSaves;

/// Fills [`SaveGames`] from the save folder, unless it holds examples or there is no folder.
fn scan_saves(
    mut saves: ResMut<SaveGames>,
    settings: Option<Res<crate::sim_bridge::HostSettings>>,
    examples: Option<Res<ExampleSaves>>,
    player: Option<Res<crate::sim_bridge::LocalPlayer>>,
) {
    let Some(dir) = settings.and_then(|s| s.save_dir.clone()) else {
        return;
    };
    if examples.is_some() {
        return;
    }
    let me = player.map(|p| p.0);
    let (found, errors) = arcanum_sim::list_saves(&dir);
    for e in errors {
        warn!("cannot read a save: {e}");
    }
    saves.0 = found
        .into_iter()
        .map(|listing| {
            let pc = me
                .and_then(|p| listing.save.players.get(&p))
                .or_else(|| listing.save.players.values().next());
            SaveSummary {
                name: pc.map_or_else(|| "?".into(), |p| p.blueprint.sheet.name.clone()),
                path: Some(listing.path),
                level: pc.map_or(1, |p| p.blueprint.sheet.level.max(1) as u32),
                location: listing.save.map.clone(),
                saved_at: format_time(listing.modified),
            }
        })
        .collect();
}

/// `d Mon yyyy hh:mm` in UTC.
fn format_time(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{day} {} {year} {:02}:{:02}",
        MONTHS[(month - 1) as usize],
        rest / 3600,
        rest / 60 % 60
    )
}

/// Selection and the info toggle. Reset each time the screen opens.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadGameState {
    pub selected: Option<usize>,
    /// Shows the selected save's details over its screenshot.
    pub show_info: bool,
}

/// What a Load Game button does.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadGameAction {
    Load,
    Back,
    Delete,
    ToggleInfo,
    Select(usize),
}

/// Root of the Load Game screen (rebuilt when the list or selection changes).
#[derive(Component)]
pub struct LoadGameScreen;

/// A row in the save list.
#[derive(Component, Debug)]
pub struct SaveRow(pub usize);

/// The details shown over the screenshot when the info toggle is on.
#[derive(Component)]
pub struct SaveInfo;

/// Shown in the list when there are no saves. Not in the game's text.
pub const NO_SAVES: &str = "No saved games";

/// Save list entries.
pub const ROW_COLOR: Color = Color::srgb_u8(214, 186, 128);

// Layout in original 800x600 pixels; everything is multiplied by ORIGINAL_SCALE.
/// Inside the list box (background 33,111 - 209,343), offset to the screen.
const LIST: Rect = Rect {
    min: Vec2::new(38.0, 116.0 + TOP_BAR_HEIGHT),
    max: Vec2::new(204.0, 338.0 + TOP_BAR_HEIGHT),
};
/// Portrait ring over the socket above the list.
const LENS_AT: Vec2 = Vec2::new(84.0, 8.0 + TOP_BAR_HEIGHT);
const LENS_SIZE: Vec2 = Vec2::new(89.0, 89.0);
/// Screenshot frame over the oval window.
const SHOT: Rect = Rect {
    min: Vec2::new(281.0, 55.0 + TOP_BAR_HEIGHT),
    max: Vec2::new(749.0, 355.0 + TOP_BAR_HEIGHT),
};
/// Buttons: positions from the original's screen table.
const DELETE_AT: Vec2 = Vec2::new(115.0, 396.0);
const DELETE_SIZE: Vec2 = Vec2::new(38.0, 37.0);
const TOGGLE_AT: Vec2 = Vec2::new(499.0, 82.0);
const TOGGLE_SIZE: Vec2 = Vec2::new(33.0, 33.0);

/// Art handles, loaded at startup so the screen opens instantly.
#[derive(Resource)]
struct LoadGameArt {
    background: Handle<Image>,
    lens: Handle<Image>,
    shot_cover: Handle<Image>,
    delete: Handle<ArtFrames>,
    toggle: Handle<ArtFrames>,
}

/// Present when the screen needs (re)building.
#[derive(Resource)]
struct RebuildLoadGame;

pub struct LoadGamePlugin;

impl Plugin for LoadGamePlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<MenuWindowPlugin>() {
            app.add_plugins(MenuWindowPlugin);
        }
        app.init_resource::<SaveGames>()
            .init_resource::<LoadGameState>()
            .add_systems(Startup, load_art)
            .add_systems(OnEnter(AppState::LoadGame), (scan_saves, enter).chain())
            .add_systems(OnExit(AppState::LoadGame), |mut commands: Commands| {
                commands.remove_resource::<RebuildLoadGame>();
            })
            .add_systems(
                Update,
                (
                    request_rebuild,
                    build_screen.run_if(resource_exists::<RebuildLoadGame>),
                    show_toggle_frame,
                )
                    .chain()
                    .run_if(in_state(AppState::LoadGame)),
            )
            .add_observer(on_activate);
    }
}

fn load_art(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(LoadGameArt {
        background: assets.load(interface::SAVE_LOAD_BACK),
        lens: assets.load(interface::SAVE_LOAD_LENS),
        shot_cover: assets.load(interface::SAVE_SHOT_COVER),
        delete: assets.load(interface::DELETE_BUTTON),
        toggle: assets.load(interface::TEXT_TOGGLE),
    });
}

fn enter(mut commands: Commands, saves: Res<SaveGames>, mut state: ResMut<LoadGameState>) {
    // The newest save starts selected, as there is nothing else to show otherwise.
    *state = LoadGameState {
        selected: (!saves.0.is_empty()).then_some(0),
        show_info: false,
    };
    commands.insert_resource(RebuildLoadGame);
}

/// Rebuilds when the saves or the selection change.
fn request_rebuild(mut commands: Commands, saves: Res<SaveGames>, state: Res<LoadGameState>) {
    if saves.is_changed() || state.is_changed() {
        commands.insert_resource(RebuildLoadGame);
    }
}

#[allow(clippy::too_many_arguments)]
fn build_screen(
    mut commands: Commands,
    existing: Query<Entity, With<LoadGameScreen>>,
    art: Res<LoadGameArt>,
    frame: Res<WindowArt>,
    saves: Res<SaveGames>,
    state: Res<LoadGameState>,
    fonts: Option<Res<ArcanumFonts>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<RebuildLoadGame>();
    let fonts = fonts.map(|f| f.clone()).unwrap_or_default();
    let selected = state.selected.and_then(|i| saves.0.get(i).map(|s| (i, s)));

    commands
        .spawn((
            LoadGameScreen,
            DespawnOnExit(AppState::LoadGame),
            screen_root(),
        ))
        .with_children(|screen| {
            spawn_frame(
                screen,
                &frame,
                &fonts,
                art.background.clone(),
                LoadGameAction::Back,
                (LoadGameAction::Load, HoverHelp(7020, "Load Game")),
            );
            // The portrait would go under the ring; saves have no portrait yet.
            screen.spawn((
                ImageNode::new(art.lens.clone()),
                at(rect(LENS_AT, LENS_SIZE)),
            ));
            // The screenshot would go under the frame; saves have no screenshot yet.
            screen.spawn((ImageNode::new(art.shot_cover.clone()), at(SHOT)));

            // Save list.
            screen
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: crate::menu_window::px(2.0),
                    overflow: Overflow::clip(),
                    ..at(LIST)
                })
                .with_children(|list| {
                    if saves.0.is_empty() {
                        let mut empty = list.spawn(TextColor(ROW_COLOR));
                        fonts.insert_text(&mut empty, NO_SAVES, UiFont::Small);
                    }
                    for (i, save) in saves.0.iter().enumerate() {
                        let is_selected = Some(i) == state.selected;
                        let colour = if is_selected { HOVER_COLOR } else { ROW_COLOR };
                        let mut row = list.spawn((
                            SaveRow(i),
                            LoadGameAction::Select(i),
                            Button,
                            ActivateOnPress,
                            Hovered::default(),
                            HoverText {
                                normal: colour,
                                hover: HOVER_COLOR,
                            },
                            TextColor(colour),
                        ));
                        fonts.insert_text(&mut row, &save.name, UiFont::Small);
                    }
                });

            // Details over the screenshot.
            if state.show_info
                && let Some((_, save)) = selected
            {
                screen
                    .spawn((
                        SaveInfo,
                        // Covers the oval, including the info toggle: let the pointer through.
                        bevy::picking::Pickable::IGNORE,
                        Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            row_gap: crate::menu_window::px(4.0),
                            ..at(SHOT)
                        },
                    ))
                    .with_children(|info| {
                        for line in [
                            save.name.clone(),
                            format!("Level {}", save.level),
                            save.location.clone(),
                            save.saved_at.clone(),
                        ] {
                            let mut text = info.spawn(TextColor(INFO_COLOR));
                            fonts.insert_text(&mut text, &line, UiFont::Small);
                        }
                    });
            }

            screen.spawn(image_button(
                LoadGameAction::Delete,
                HoverHelp(7022, "Delete Save Game"),
                &art.delete,
                rect(DELETE_AT, DELETE_SIZE),
            ));
            screen.spawn((
                LoadGameAction::ToggleInfo,
                HoverHelp(7025, "Toggle Info Display"),
                Button,
                ActivateOnPress,
                Hovered::default(),
                ImageNode::default(),
                at(rect(TOGGLE_AT, TOGGLE_SIZE)),
            ));
        });
}

/// The info toggle shows frame 1 while on, frame 0 while off.
fn show_toggle_frame(
    art: Res<LoadGameArt>,
    frames: Res<Assets<ArtFrames>>,
    state: Res<LoadGameState>,
    mut toggles: Query<(&LoadGameAction, &mut ImageNode)>,
) {
    let Some(set) = frames.get(&art.toggle) else {
        return;
    };
    for (action, mut image) in &mut toggles {
        if *action != LoadGameAction::ToggleInfo {
            continue;
        }
        if let Some(frame) = set.frame(state.show_info as usize)
            && image.image != *frame
        {
            image.image = frame.clone();
        }
    }
}

fn on_activate(
    activate: On<Activate>,
    actions: Query<&LoadGameAction>,
    mut commands: Commands,
    mut saves: ResMut<SaveGames>,
    mut state: ResMut<LoadGameState>,
    mut next_state: ResMut<NextState<AppState>>,
    sounds: Option<Res<MenuSounds>>,
) {
    let Ok(&action) = actions.get(activate.entity) else {
        return;
    };
    let selected = state.selected.filter(|i| *i < saves.0.len());
    // Load and Delete need a save; without one they do nothing (and stay silent).
    if matches!(action, LoadGameAction::Load | LoadGameAction::Delete) && selected.is_none() {
        return;
    }
    if let Some(sounds) = &sounds {
        match action {
            LoadGameAction::Select(_) => sounds.play_click(&mut commands),
            _ => sounds.play_button_press(&mut commands),
        }
    }
    match action {
        LoadGameAction::Back => next_state.set(AppState::SinglePlayer),
        LoadGameAction::Load => {
            let Some(path) = selected.and_then(|i| saves.0[i].path.clone()) else {
                return;
            };
            commands.queue(move |world: &mut World| crate::sim_bridge::boot_save(world, &path));
            commands.insert_resource(LoadRequest::new(AppState::InGame).after_save_boots());
            next_state.set(AppState::Loading);
        }
        LoadGameAction::Delete => {
            if let Some(i) = selected {
                // A confirmation (`MainMenu.mes` 5150) belongs here once saves are real files.
                saves.0.remove(i);
                state.selected = (!saves.0.is_empty()).then(|| i.min(saves.0.len() - 1));
            }
        }
        LoadGameAction::ToggleInfo => state.show_info = !state.show_info,
        LoadGameAction::Select(i) => {
            if state.selected != Some(i) {
                state.selected = Some(i);
            }
        }
    }
}

/// True once the Load Game screen has been built.
pub fn load_game_ready(screens: Query<(), With<LoadGameScreen>>) -> bool {
    !screens.is_empty()
}

/// Example saves for screenshots (`DEBUG_ARCANUM_SCREEN=loadgame`).
pub fn sample_saves() -> SaveGames {
    let save = |name: &str, level, location: &str, saved_at: &str| SaveSummary {
        name: name.into(),
        path: None,
        level,
        location: location.into(),
        saved_at: saved_at.into(),
    };
    SaveGames(vec![
        save("Before the bridge", 7, "Tarant", "13 Nov 2026 18:40"),
        save("Shrouded Hills", 3, "Shrouded Hills", "12 Nov 2026 21:05"),
        save("Crash site", 1, "Crash site", "12 Nov 2026 20:11"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_the_original_screen() {
        // Buttons sit inside the screen.
        for (at, size) in [(DELETE_AT, DELETE_SIZE), (TOGGLE_AT, TOGGLE_SIZE)] {
            assert!(at.x + size.x <= 800.0 && at.y + size.y <= 600.0, "{at:?}");
        }
        // The screenshot frame fills the oval window (468x300).
        assert_eq!(SHOT.size(), Vec2::new(468.0, 300.0));
    }

    #[test]
    fn save_times_read_as_dates() {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_795_000_000);
        assert_eq!(format_time(t), "18 Nov 2026 11:06");
        assert_eq!(format_time(std::time::UNIX_EPOCH), "1 Jan 1970 00:00");
    }
}
