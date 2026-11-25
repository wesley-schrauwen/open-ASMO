//! Conversations on screen (Arcanum.exe: the options panel "TC" 0x4c9280 / 0x4c9810 / 0x4c9a10,
//! floating lines 0x568540 / 0x4d5fe0, voices 0x5688d0). The host runs the conversation
//! ([`arcanum_sim::dialogue`]).
//!
//! - **Talking:** a click on an NPC walks the PC to it and asks to talk
//!   ([`crate::actors`]).
//! - **Floating lines** ([`arcanum_sim::LedgerDelta::FloatText`]): over the speaker's head in
//!   Flare12, white for NPCs and green for the PC's choice, centred and wrapped to
//!   [`FLOAT_WIDTH`] original pixels. A new line from the same speaker replaces the old one. The
//!   line of the NPC the player is talking to stays until the conversation moves on or ends
//!   (the original's duration -2); others last [`float_secs`].
//! - **Voices:** the line's speech file plays at the Voice volume, falling back from `_f` to
//!   `_m` (0x5688d0); a new line stops the one playing.
//! - **The options panel:** a black panel at (200, 419), 400x100, rows 19 pixels apart from y 3,
//!   white, cyan under the cursor, blue pressed; a click chooses ([`arcanum_sim::PlayerIntent::Choose`]).
//!   Escape leaves the conversation. While talking, clicks do not reach the world.
//!
//! Builder's calls: how long lines float ([`float_secs`]); the float's height over the head;
//! Escape leaving; world clicks blocked while talking. Not checked against the original by eye.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, ActivateOnPress, Button};

use arcanum_sim::{FloatColour, LedgerDelta, NetId, PlayerIntent};

use crate::actors::Actor;
use crate::fonts::{ArcanumFonts, UiFont};
use crate::main_menu::AppState;
use crate::menu_window::{at, px, rect};
use crate::sim_bridge::HostLink;

/// The options panel, in screen pixels (0x4c9280 on an 800x600 screen).
pub const PANEL: Rect = Rect { min: Vec2::new(200.0, 419.0), max: Vec2::new(600.0, 519.0) };
/// Rows are this far apart, from [`ROW_TOP`].
pub const ROW_HEIGHT: f32 = 19.0;
pub const ROW_TOP: f32 = 3.0;
/// Floating lines wrap to this width (original pixels; the floater's surface is 200 wide).
pub const FLOAT_WIDTH: f32 = 200.0;
const WHITE: Color = Color::srgb_u8(255, 255, 255);
const CYAN: Color = Color::srgb_u8(0, 255, 255);
const BLUE: Color = Color::srgb_u8(0, 0, 255);
const GREEN: Color = Color::srgb_u8(0, 255, 0);

/// How long a line floats, in seconds: 3, plus one for every 15 characters.
pub fn float_secs(text: &str) -> f32 {
    3.0 + text.chars().count() as f32 / 15.0
}

/// The player's conversation as the host last told it.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct DialogueState {
    pub npc: Option<NetId>,
    pub options: Vec<String>,
    /// Options or the end came in since the panel was last drawn.
    pub changed: bool,
}

impl DialogueState {
    /// Takes the host's deltas.
    pub fn receive(&mut self, delta: &LedgerDelta) {
        match delta {
            LedgerDelta::DialogueOptions { npc, options } => *self = Self { npc: Some(*npc), options: options.clone(), changed: true },
            LedgerDelta::DialogueEnded => *self = Self { npc: None, options: Vec::new(), changed: true },
            _ => {}
        }
    }

    pub fn talking(&self) -> bool {
        self.npc.is_some()
    }
}

/// Lines to float, from the host.
#[derive(Resource, Debug, Default)]
pub struct Floats(pub Vec<(NetId, String, FloatColour, Option<String>)>);

/// A floating line over `speaker`; `until` in real seconds (`None`: until replaced).
#[derive(Component, Debug)]
struct FloatLine {
    speaker: NetId,
    until: Option<f32>,
}

#[derive(Component)]
struct OptionsPanel;

#[derive(Component, Debug, Clone, Copy)]
struct OptionRow(usize);

/// The voice line playing.
#[derive(Component)]
struct DialogueVoice;

pub struct DialogueUiPlugin;

impl Plugin for DialogueUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DialogueState>()
            .init_resource::<Floats>()
            .add_systems(OnExit(AppState::InGame), |mut state: ResMut<DialogueState>| *state = DialogueState::default())
            .add_systems(PreUpdate, leave_on_escape.run_if(in_state(AppState::InGame)).after(bevy::input::InputSystems))
            .add_systems(Update, (spawn_floats, expire_floats, place_floats, show_panel, colour_rows).chain().run_if(in_state(AppState::InGame)))
            .add_observer(on_option);
    }
}

fn send(link: Option<&HostLink>, intent: PlayerIntent) {
    if let Some(link) = link
        && let Err(e) = link.0.send_intent(intent)
    {
        warn!("cannot reach the host: {e}");
    }
}

/// Escape leaves the conversation.
fn leave_on_escape(mut keys: ResMut<ButtonInput<KeyCode>>, state: Res<DialogueState>, link: Option<Res<HostLink>>) {
    if state.talking() && keys.clear_just_pressed(KeyCode::Escape) {
        send(link.as_deref(), PlayerIntent::EndTalk);
    }
}

/// Floats the host's lines and plays their voices.
#[allow(clippy::too_many_arguments)]
fn spawn_floats(
    mut commands: Commands,
    mut floats: ResMut<Floats>,
    state: Res<DialogueState>,
    existing: Query<(Entity, &FloatLine)>,
    voices: Query<Entity, With<DialogueVoice>>,
    fonts: Option<Res<ArcanumFonts>>,
    (assets, time): (Res<AssetServer>, Res<Time<Real>>),
    vfs: Option<Res<crate::vfs_asset::VfsResource>>,
    settings: Option<Res<crate::settings::GameSettings>>,
) {
    let fonts = fonts.map(|f| f.clone()).unwrap_or_default();
    for (speaker, text, colour, voice) in floats.0.drain(..) {
        for (entity, line) in &existing {
            if line.speaker == speaker {
                commands.entity(entity).despawn();
            }
        }
        let sticky = state.npc == Some(speaker);
        let until = (!sticky).then(|| time.elapsed_secs() + float_secs(&text));
        let colour = match colour {
            FloatColour::White => WHITE,
            FloatColour::Green => GREEN,
            FloatColour::Red => Color::srgb_u8(255, 0, 0),
            FloatColour::Yellow => Color::srgb_u8(255, 255, 0),
        };
        commands
            .spawn((
                FloatLine { speaker, until },
                DespawnOnExit(AppState::InGame),
                bevy::picking::Pickable::IGNORE,
                Visibility::Hidden,
                Node { position_type: PositionType::Absolute, width: px(FLOAT_WIDTH), justify_content: JustifyContent::Center, ..default() },
            ))
            .with_children(|b| {
                let mut t = b.spawn(TextColor(colour));
                let scale = crate::fonts::ORIGINAL_SCALE;
                fonts.insert_block(&mut t, &text, UiFont::Body, FLOAT_WIDTH * scale, None, crate::bitmap_text::Align::Center);
            });
        // The voice: `_f`, else `_m` (0x5688d0).
        let Some(path) = voice else { continue };
        let exists = |p: &str| vfs.as_ref().and_then(|v| v.0.read().ok()).is_some_and(|v| v.exists(p));
        let path = if exists(&path) { Some(path) } else { Some(path.replace("_f.mp3", "_m.mp3")).filter(|p| exists(p)) };
        let (Some(path), Some(settings)) = (path, settings.as_deref()) else { continue };
        for voice in &voices {
            commands.entity(voice).despawn();
        }
        let line = crate::audio::play_voice(&mut commands, settings, assets.load(path));
        commands.entity(line).insert((DialogueVoice, DespawnOnExit(AppState::InGame)));
    }
}

/// Lines whose time is up go; the conversation's NPC's line goes when it ends.
fn expire_floats(mut commands: Commands, lines: Query<(Entity, &FloatLine)>, state: Res<DialogueState>, time: Res<Time<Real>>, mut was: Local<Option<NetId>>) {
    let ended = was.filter(|&npc| state.npc != Some(npc));
    *was = state.npc;
    let now = time.elapsed_secs();
    for (entity, line) in &lines {
        if line.until.is_some_and(|t| now >= t) || (line.until.is_none() && ended == Some(line.speaker)) {
            commands.entity(entity).despawn();
        }
    }
}

/// Keeps each line over its speaker's head.
fn place_floats(
    mut lines: Query<(&FloatLine, &mut Node, &mut Visibility, &ComputedNode)>,
    actors: Query<(&Actor, &GlobalTransform, &Sprite)>,
    images: Res<Assets<Image>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    ui_scale: Res<UiScale>,
) {
    let Some((camera, at)) = cameras.iter().next() else { return };
    for (line, mut node, mut visibility, computed) in &mut lines {
        let speaker = actors.iter().find(|a| a.0.net_id == line.speaker);
        let Some((_, transform, sprite)) = speaker else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        let height = images.get(&sprite.image).map_or(80.0, |i| i.size().y as f32);
        let head = transform.translation().truncate() + Vec2::new(0.0, height / 2.0 + 8.0);
        let Ok(point) = camera.world_to_viewport(at, head.extend(0.0)) else { continue };
        let scale = ui_scale.0.max(f32::EPSILON);
        let size = computed.size() * computed.inverse_scale_factor();
        let (left, top) = (Val::Px(point.x / scale - size.x / scale / 2.0), Val::Px(point.y / scale - size.y / scale));
        if node.left != left || node.top != top {
            (node.left, node.top) = (left, top);
        }
        visibility.set_if_neq(Visibility::Inherited);
    }
}

/// Draws the options panel when the options change.
fn show_panel(mut commands: Commands, mut state: ResMut<DialogueState>, panels: Query<Entity, With<OptionsPanel>>, fonts: Option<Res<ArcanumFonts>>) {
    if !state.changed {
        return;
    }
    state.changed = false;
    for panel in &panels {
        commands.entity(panel).despawn();
    }
    if state.options.is_empty() {
        return;
    }
    let fonts = fonts.map(|f| f.clone()).unwrap_or_default();
    commands
        .spawn((
            OptionsPanel,
            DespawnOnExit(AppState::InGame),
            bevy::picking::Pickable::IGNORE,
            Node { position_type: PositionType::Absolute, top: Val::Px(0.0), left: Val::Percent(50.0), margin: UiRect::left(px(-400.0)), width: px(800.0), height: px(600.0), ..default() },
        ))
        .with_children(|screen| {
            screen.spawn((BackgroundColor(Color::BLACK), at(PANEL)));
            for (i, option) in state.options.iter().enumerate() {
                let row = rect(PANEL.min + Vec2::new(6.0, ROW_TOP + ROW_HEIGHT * i as f32), Vec2::new(PANEL.width() - 12.0, ROW_HEIGHT));
                screen.spawn((OptionRow(i), Button, ActivateOnPress, Hovered::default(), Node { overflow: Overflow::clip(), ..at(row) })).with_children(|cell| {
                    let mut t = cell.spawn((TextColor(WHITE), bevy::picking::Pickable::IGNORE));
                    fonts.insert_text(&mut t, option, UiFont::Body);
                });
            }
        });
}

/// White rows, cyan under the cursor, blue while pressed.
fn colour_rows(rows: Query<(&Hovered, &Children, Has<bevy::ui::Pressed>), With<OptionRow>>, mut colours: Query<&mut TextColor>) {
    for (hovered, children, pressed) in &rows {
        let colour = match (pressed, hovered.0) {
            (true, _) => BLUE,
            (false, true) => CYAN,
            _ => WHITE,
        };
        for &child in children {
            if let Ok(mut c) = colours.get_mut(child) {
                c.set_if_neq(TextColor(colour));
            }
        }
    }
}

fn on_option(activate: On<Activate>, rows: Query<&OptionRow>, mut state: ResMut<DialogueState>, link: Option<Res<HostLink>>) {
    let Ok(&OptionRow(option)) = rows.get(activate.entity) else { return };
    send(link.as_deref(), PlayerIntent::Choose { option });
    // No second click until the host answers.
    state.options.clear();
    state.changed = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_follows_the_host() {
        let mut state = DialogueState::default();
        assert!(!state.talking());
        state.receive(&LedgerDelta::DialogueOptions { npc: NetId(4), options: vec!["Hi.".into()] });
        assert_eq!((state.npc, state.options.len(), state.changed), (Some(NetId(4)), 1, true));
        state.receive(&LedgerDelta::DialogueEnded);
        assert!(!state.talking() && state.options.is_empty());
        assert_eq!(float_secs("fifteen letters"), 4.0);
    }
}
