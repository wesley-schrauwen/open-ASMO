//! The character editor's stats page, shown after character creation (Arcanum.exe window 14
//! opens the in-game character screen, 0x5597c0, over the menu; see NOTES.md).
//!
//! Background `Char_Maint.art` (800x400 at y = 41) in the shared menu frame. Positions are the
//! original's window coordinates (screen y = window y + 41):
//! - Info panel: name (212,15); Level / Points / XP Next (212, 34/53/72); Race / Gender / Age
//!   (337, 34/53/72). Labels from `mes/charedit.mes` 0-6.
//! - Primary stats: values centred at x 211 (ST CN DX BE) and 350 (IN WP PE CH), rows y 115-235;
//!   minus/plus buttons (`Char_Minus` / `Char_Plus`, 19x19) beside them.
//! - Hit points (heart) and fatigue (cross): values centred at (447,143) and (447,213), with
//!   minus/plus buttons; each point buys 4.
//! - "Derived Stats" (charedit 7) and "Resistance" (8) with their rows (`stat.mes` 8-15,
//!   charedit 12-16), in `Flare12Font`.
//! - Refused clicks show the original's message (charedit 17-20) in the help box.
//!
//! - Skills tab (the first of the tabs at (527/595/663, 11), art 34-36): the skills panel
//!   (`Skills_Window`, art 29, at (503,63)) with the group buttons (combat, thieving, social,
//!   technological; art 302-305 at x 524/578/632/688, y 80) and four rows 66 apart: the name
//!   (and training rank, " (Expert)") at (520,126), a gauge (`SkilGauge`, art 623, at
//!   (562,150)) filled `120 x level / 20` wide, and minus/plus buttons (art 625/624) at 521 and
//!   695, y 147 (Arcanum.exe 0x55bab0, 0x55bf20). No numbers are shown.
//!
//! Back returns to character creation with the character kept; Next keeps the finished
//! character and loads the game (the original shows one more screen, window 15, first). The
//! tech and spells tabs are drawn but do nothing yet.
//!
//! In game the same screen ([`CharacterPanel`]) opens between the in-game bars from the top
//! bar's character button, without the menu frame. Its changes go through the host
//! ([`SheetHost`], [`arcanum_sim::sheet`]): minus goes back to the values at opening, closing
//! (the button again, or Escape) keeps them.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, ActivateOnPress, Button};

use crate::art_loader::ArtFrames;
use crate::audio::MenuSounds;
use crate::character_host::CharacterHost;
use crate::pc_sheet::SheetHost;
use arcanum_sim::SheetRequest;
use crate::create_character::CreationData;
use arcanum_rules::{CharacterAction, MAX_SKILL_LEVEL, Message, PRIMARY_STATS, Pool, RESISTANCES, SKILL_GROUP_SIZE, XP_FOR_LEVEL_2};
use crate::fonts::{ArcanumFonts, UiFont};
use crate::loading::LoadRequest;
use crate::main_menu::AppState;
use crate::menu_window::{HelpMessage, HoverHelp, MenuWindowPlugin, SCREEN_WHITE, TOP_BAR_HEIGHT, WindowArt, at, rect, screen_root, spawn_frame};
use crate::resource_pack::paths::interface;
use crate::widgets::FrameButton;

/// What a character editor button does.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorAction {
    Back,
    Next,
    /// Primary stat (stat.mes order: ST DX CN BE IN PE WP CH) up or down.
    Stat(usize, i32),
    HitPoints(i32),
    Fatigue(i32),
    /// A skill row (0-3 in the shown group) up or down.
    SkillRow(usize, i32),
    /// Shows a skill group: combat, thieving, social, technological.
    SkillGroup(usize),
}

/// The skill group the skills tab shows.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShownSkillGroup(pub usize);

/// Root of the editor screen.
#[derive(Component)]
pub struct CharacterEditorScreen;

/// Root of the values (redrawn when the character changes).
#[derive(Component)]
pub struct EditorContent;

/// Marks texts, for tests.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorText {
    Name,
    Level,
    Points,
    XpNext,
    Race,
    Gender,
    Age,
    Stat(usize),
    HitPoints,
    Fatigue,
    Derived(usize),
    Resist(usize),
    /// A skill's name row (`mes/skill.mes` index).
    SkillName(usize),
}

/// A skill's gauge, with its fill width in the original's pixels.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillGauge {
    pub skill: usize,
    pub width: u32,
}

/// A primary stat raised above its starting value (Arcanum.exe font 0x64c7a0).
pub const RAISED: Color = Color::srgb_u8(0, 255, 0);
/// The name in the info panel.
pub const NAME_COLOR: Color = Color::srgb_u8(255, 210, 0);

#[derive(Resource)]
struct EditorArt {
    background: Handle<Image>,
    plus: Handle<ArtFrames>,
    minus: Handle<ArtFrames>,
    tabs: [Handle<ArtFrames>; 3],
    skills_window: Handle<Image>,
    groups: [Handle<ArtFrames>; 4],
    gauge: Handle<Image>,
    skill_plus: Handle<ArtFrames>,
    skill_minus: Handle<ArtFrames>,
}

/// Window coordinates (the background starts at screen y = 41).
fn win(x: f32, y: f32, w: f32, h: f32) -> Rect {
    rect(Vec2::new(x, y + TOP_BAR_HEIGHT), Vec2::new(w, h))
}

/// A box of width `w` centred on `x`.
fn centred_on(x: f32, y: f32, w: f32, h: f32) -> Rect {
    win(x - w / 2.0, y, w, h)
}

const BUTTON: f32 = 19.0;
/// Rows of the primary stats, and which stat sits in each (left column ST CN DX BE, right
/// column IN WP PE CH; stat.mes indices).
const STAT_ROWS: [f32; 4] = [115.0, 155.0, 195.0, 235.0];
const LEFT_STATS: [usize; 4] = [0, 2, 1, 3];
const RIGHT_STATS: [usize; 4] = [4, 6, 5, 7];
const DERIVED_ROWS: [f32; 4] = [308.0, 326.0, 344.0, 362.0];
const RESIST_ROWS: [f32; RESISTANCES] = [301.0, 317.0, 333.0, 349.0, 365.0];
const TAB_X: [f32; 3] = [527.0, 595.0, 663.0];
const GROUP_X: [f32; 4] = [524.0, 578.0, 632.0, 688.0];
const GROUP_SIZE: [(f32, f32); 4] = [(32.0, 32.0), (33.0, 32.0), (34.0, 32.0), (32.0, 32.0)];
/// Skill rows are 66 pixels apart.
const SKILL_ROW: f32 = 66.0;
const GAUGE_WIDTH: f32 = 120.0;
const GAUGE_HEIGHT: f32 = 27.0;

/// The gauge's fill width for a skill level: `120 x level / 20`, rounded down (0x55bf20).
pub fn gauge_width(level: i32) -> u32 {
    (GAUGE_WIDTH as i32 * level.clamp(0, MAX_SKILL_LEVEL) / MAX_SKILL_LEVEL) as u32
}

pub struct CharacterEditorPlugin;

impl Plugin for CharacterEditorPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<MenuWindowPlugin>() {
            app.add_plugins(MenuWindowPlugin);
        }
        app.add_systems(Startup, |mut commands: Commands, assets: Res<AssetServer>| {
            commands.insert_resource(EditorArt {
                background: assets.load(interface::CHAR_MAINT),
                plus: assets.load(interface::CHAR_PLUS),
                minus: assets.load(interface::CHAR_MINUS),
                tabs: interface::CHAR_TABS.map(|p| assets.load(p)),
                skills_window: assets.load(interface::SKILLS_WINDOW),
                groups: interface::SKILL_GROUPS.map(|p| assets.load(p)),
                gauge: assets.load(interface::SKILL_GAUGE),
                skill_plus: assets.load(interface::SKILL_PLUS),
                skill_minus: assets.load(interface::SKILL_MINUS),
            });
        })
        .add_systems(OnEnter(AppState::CharacterEditor), (crate::create_character::load_creation_data, build_screen).chain())
        .add_systems(Update, show_values.run_if(in_state(AppState::CharacterEditor)))
        .init_resource::<CharacterPanel>()
        .init_resource::<ShownSkillGroup>()
        .add_systems(Update, show_skill_group)
        .add_systems(OnEnter(AppState::InGame), crate::create_character::load_creation_data)
        .add_systems(OnExit(AppState::InGame), |mut panel: ResMut<CharacterPanel>| *panel = CharacterPanel::default())
        .add_systems(Update, (show_panel, show_panel_values).chain().run_if(in_state(AppState::InGame)))
        .add_systems(PreUpdate, close_panel_on_escape.run_if(in_state(AppState::InGame)).after(bevy::input::InputSystems))
        .add_observer(on_activate)
        .add_observer(on_hud_button);
    }
}

fn button(action: EditorAction, frames: &Handle<ArtFrames>, x: f32, y: f32) -> impl Bundle {
    sized_button(action, frames, win(x, y, BUTTON, BUTTON))
}

fn sized_button(action: EditorAction, frames: &Handle<ArtFrames>, r: Rect) -> impl Bundle {
    (action, Button, ActivateOnPress, Hovered::default(), FrameButton::arrow(frames.clone()), ImageNode::default(), at(r))
}

fn build_screen(mut commands: Commands, art: Res<EditorArt>, frame: Res<WindowArt>, fonts: Option<Res<ArcanumFonts>>) {
    let fonts = fonts.map(|f| f.clone()).unwrap_or_default();
    commands
        .spawn((CharacterEditorScreen, DespawnOnExit(AppState::CharacterEditor), screen_root()))
        .with_children(|screen| {
            spawn_frame(
                screen,
                &frame,
                &fonts,
                art.background.clone(),
                EditorAction::Back,
                (EditorAction::Next, HoverHelp(7003, "Next Screen")),
            );
            spawn_buttons(screen, &art);
        });
}

/// The +/- buttons of the stats and pools.
fn spawn_buttons(screen: &mut ChildSpawnerCommands, art: &EditorArt) {
    for (i, &y) in STAT_ROWS.iter().enumerate() {
        let by = y + 2.0;
        screen.spawn(button(EditorAction::Stat(LEFT_STATS[i], -1), &art.minus, 170.0, by));
        screen.spawn(button(EditorAction::Stat(LEFT_STATS[i], 1), &art.plus, 224.0, by));
        screen.spawn(button(EditorAction::Stat(RIGHT_STATS[i], -1), &art.minus, 313.0, by));
        screen.spawn(button(EditorAction::Stat(RIGHT_STATS[i], 1), &art.plus, 367.0, by));
    }
    screen.spawn(button(EditorAction::HitPoints(-1), &art.minus, 409.0, 144.0));
    screen.spawn(button(EditorAction::HitPoints(1), &art.plus, 465.0, 144.0));
    screen.spawn(button(EditorAction::Fatigue(-1), &art.minus, 409.0, 214.0));
    screen.spawn(button(EditorAction::Fatigue(1), &art.plus, 465.0, 214.0));
    // The tabs: skills is shown (its pressed frame); tech and spells are not built.
    for (i, (&x, frames)) in TAB_X.iter().zip(&art.tabs).enumerate() {
        let mut look = FrameButton::arrow(frames.clone());
        if i == 0 {
            look.normal = look.pressed;
            look.hover = look.pressed;
        }
        screen.spawn((Hovered::default(), look, ImageNode::default(), at(win(x, 11.0, 39.0, 39.0))));
    }
    screen.spawn((bevy::picking::Pickable::IGNORE, ImageNode::new(art.skills_window.clone()), at(win(503.0, 63.0, 235.0, 325.0))));
    for (g, (&x, frames)) in GROUP_X.iter().zip(&art.groups).enumerate() {
        let (w, h) = GROUP_SIZE[g];
        screen.spawn(sized_button(EditorAction::SkillGroup(g), frames, win(x, 80.0, w, h)));
    }
    for row in 0..SKILL_GROUP_SIZE {
        let y = 147.0 + SKILL_ROW * row as f32;
        screen.spawn(sized_button(EditorAction::SkillRow(row, -1), &art.skill_minus, win(521.0, y, 29.0, 30.0)));
        screen.spawn(sized_button(EditorAction::SkillRow(row, 1), &art.skill_plus, win(695.0, y, 29.0, 30.0)));
    }
}

/// The shown group's button keeps its pressed frame.
fn show_skill_group(group: Res<ShownSkillGroup>, mut buttons: Query<(&EditorAction, &mut FrameButton)>, added: Query<(), Added<EditorAction>>) {
    if !group.is_changed() && added.is_empty() {
        return;
    }
    for (action, mut look) in &mut buttons {
        let EditorAction::SkillGroup(g) = *action else { continue };
        let normal = if g == group.0 { look.pressed } else { 0 };
        if look.normal != normal {
            look.normal = normal;
        }
    }
}

/// Whether the in-game character screen is open (the top bar's character button toggles it).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CharacterPanel {
    pub open: bool,
}

/// Root of the in-game character screen.
#[derive(Component)]
pub struct CharacterPanelScreen;

/// The refusal message on the in-game screen, shown over the bottom bar's message box.
#[derive(Component)]
pub struct PanelMessage;

/// Opens or closes the in-game character screen when [`CharacterPanel`] changes, telling the
/// host (opening takes the snapshot minus can go back to; closing keeps the changes).
#[allow(clippy::too_many_arguments)]
fn show_panel(
    mut commands: Commands,
    panel: Res<CharacterPanel>,
    art: Res<EditorArt>,
    data: Option<Res<CreationData>>,
    mut sheet: ResMut<SheetHost>,
    mut message: ResMut<HelpMessage>,
    roots: Query<Entity, With<CharacterPanelScreen>>,
    fonts: Option<Res<ArcanumFonts>>,
) {
    if !panel.is_changed() {
        return;
    }
    let rules = data.as_ref().map(|d| d.rules.clone()).unwrap_or_default();
    message.0 = None;
    if !panel.open {
        for root in &roots {
            commands.entity(root).despawn();
        }
        let _ = sheet.request(&rules, SheetRequest::Close);
        return;
    }
    if !roots.is_empty() {
        return;
    }
    let _ = sheet.request(&rules, SheetRequest::Open);
    let fonts = fonts.map(|f| f.clone()).unwrap_or_default();
    commands
        .spawn((
            CharacterPanelScreen,
            DespawnOnExit(AppState::InGame),
            bevy::picking::Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Percent(50.0),
                margin: UiRect::left(crate::menu_window::px(-400.0)),
                width: crate::menu_window::px(800.0),
                height: crate::menu_window::px(600.0),
                ..default()
            },
        ))
        .with_children(|screen| {
            screen.spawn((ImageNode::new(art.background.clone()), at(rect(Vec2::new(0.0, TOP_BAR_HEIGHT), Vec2::new(800.0, 400.0)))));
            spawn_buttons(screen, &art);
            // Assumed: the bottom bar's message box (its black window) shows the refusal.
            screen.spawn((bevy::picking::Pickable::IGNORE, Node { justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..at(rect(Vec2::new(200.0, 500.0), Vec2::new(400.0, 40.0))) })).with_children(|b| {
                let mut text = b.spawn((PanelMessage, TextColor(crate::menu_window::INFO_COLOR)));
                fonts.insert_text(&mut text, "", UiFont::Small);
            });
        });
}

/// Redraws the in-game screen's values when the sheet changes, and its refusal message.
#[allow(clippy::too_many_arguments)]
fn show_panel_values(
    mut commands: Commands,
    sheet: Res<SheetHost>,
    data: Option<Res<CreationData>>,
    added: Query<(), Added<CharacterPanelScreen>>,
    contents: Query<Entity, With<EditorContent>>,
    roots: Query<Entity, With<CharacterPanelScreen>>,
    fonts: Option<Res<ArcanumFonts>>,
    message: Res<HelpMessage>,
    mut texts: Query<(Option<&mut crate::bitmap_text::BitmapText>, Option<&mut Text>), With<PanelMessage>>,
    (art, group): (Res<EditorArt>, Res<ShownSkillGroup>),
) {
    if message.is_changed() {
        let wanted = message.0.clone().unwrap_or_default();
        for (bitmap, text) in &mut texts {
            if let Some(mut b) = bitmap {
                b.text = wanted.clone();
            } else if let Some(mut t) = text {
                t.0 = wanted.clone();
            }
        }
    }
    let (Some(data), Ok(root), Some(c)) = (data, roots.single(), sheet.sheet()) else { return };
    if !sheet.is_changed() && !group.is_changed() && added.is_empty() {
        return;
    }
    for old in &contents {
        commands.entity(old).despawn();
    }
    let fonts = fonts.map(|f| f.clone()).unwrap_or_default();
    draw_values(&mut commands, root, c, &data, &fonts, &art, group.0);
}

/// Escape closes the in-game character screen (before it would leave the game).
fn close_panel_on_escape(mut keys: ResMut<ButtonInput<KeyCode>>, mut panel: ResMut<CharacterPanel>) {
    if panel.open && keys.clear_just_pressed(KeyCode::Escape) {
        panel.open = false;
    }
}

/// The top bar's character button toggles the screen.
fn on_hud_button(activate: On<Activate>, buttons: Query<&crate::hud::HudAction>, mut panel: ResMut<CharacterPanel>) {
    if buttons.get(activate.entity) == Ok(&crate::hud::HudAction::Character) {
        panel.open = !panel.open;
    }
}

/// (Re)draws every value when the character changes or the screen opens.
#[allow(clippy::too_many_arguments)]
fn show_values(
    mut commands: Commands,
    host: Res<CharacterHost>,
    data: Option<Res<CreationData>>,
    added: Query<(), Added<CharacterEditorScreen>>,
    contents: Query<Entity, With<EditorContent>>,
    roots: Query<Entity, With<CharacterEditorScreen>>,
    fonts: Option<Res<ArcanumFonts>>,
    (art, group): (Res<EditorArt>, Res<ShownSkillGroup>),
) {
    let Some(data) = data else { return };
    if !host.is_changed() && !group.is_changed() && added.is_empty() {
        return;
    }
    let Ok(root) = roots.single() else { return };
    for old in &contents {
        commands.entity(old).despawn();
    }
    let fonts = fonts.map(|f| f.clone()).unwrap_or_default();
    draw_values(&mut commands, root, host.sheet(), &data, &fonts, &art, group.0);
}

/// Spawns every value of `c` under `root` (an 800x600 screen).
#[allow(clippy::too_many_arguments)]
fn draw_values(commands: &mut Commands, root: Entity, c: &arcanum_rules::CharacterSheet, data: &CreationData, fonts: &ArcanumFonts, art: &EditorArt, group: usize) {
    let rules = &data.rules;
    let label = |id: u32, fallback: &str| data.charedit.get(id).unwrap_or(fallback).to_string();
    let stat_name = |id: u32| data.stats.get(id).unwrap_or("?").to_string();
    let stats = rules.stats(c);
    let derived = rules.derived(c);
    let resist = rules.resistances(c);
    let race = data.menu.get(720 + u32::from(c.race)).unwrap_or("").to_string();
    let gender = data.menu.get(740 + u32::from(c.gender)).unwrap_or("").to_string();

    let content = commands
        .spawn((
            EditorContent,
            bevy::picking::Pickable::IGNORE,
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        ))
        .with_children(|p| {
            let mut put = |r: Rect, text: &str, which: EditorText, font: UiFont, colour: Color, centre: bool| {
                let justify = if centre { JustifyContent::Center } else { JustifyContent::FlexStart };
                p.spawn(Node { justify_content: justify, align_items: AlignItems::Center, ..at(r) }).with_children(|cell| {
                    let mut t = cell.spawn((which, TextColor(colour)));
                    fonts.insert_text(&mut t, text, font);
                });
            };
            // Info panel.
            put(win(212.0, 15.0, 240.0, 19.0), &c.name, EditorText::Name, UiFont::Small, NAME_COLOR, false);
            put(win(212.0, 34.0, 120.0, 19.0), &format!("{}: 1", label(1, "Level")), EditorText::Level, UiFont::Small, SCREEN_WHITE, false);
            put(win(212.0, 53.0, 120.0, 19.0), &format!("{}: {}", label(0, "Points"), rules.unspent(c)), EditorText::Points, UiFont::Small, SCREEN_WHITE, false);
            put(win(212.0, 72.0, 120.0, 19.0), &format!("{}: {XP_FOR_LEVEL_2}", label(2, "XP Next")), EditorText::XpNext, UiFont::Small, SCREEN_WHITE, false);
            put(win(337.0, 34.0, 130.0, 19.0), &format!("{}: {race}", label(4, "Race")), EditorText::Race, UiFont::Small, SCREEN_WHITE, false);
            put(win(337.0, 53.0, 130.0, 19.0), &format!("{}: {gender}", label(5, "Gender")), EditorText::Gender, UiFont::Small, SCREEN_WHITE, false);
            put(win(337.0, 72.0, 130.0, 19.0), &format!("{}: 20", label(6, "Age")), EditorText::Age, UiFont::Small, SCREEN_WHITE, false);
            // Primary stats.
            for (i, &y) in STAT_ROWS.iter().enumerate() {
                for (x, stat) in [(211.0, LEFT_STATS[i]), (350.0, RIGHT_STATS[i])] {
                    let colour = if c.draft.stats[stat] > 0 { RAISED } else { SCREEN_WHITE };
                    put(centred_on(x, y, 40.0, 19.0), &stats[stat].to_string(), EditorText::Stat(stat), UiFont::Small, colour, true);
                }
            }
            let pool_colour = |bought: i32| if bought > 0 { RAISED } else { SCREEN_WHITE };
            put(centred_on(447.0, 143.0, 40.0, 19.0), &derived.hit_points.to_string(), EditorText::HitPoints, UiFont::Small, pool_colour(c.draft.hit_points), true);
            put(centred_on(447.0, 213.0, 40.0, 19.0), &derived.fatigue.to_string(), EditorText::Fatigue, UiFont::Small, pool_colour(c.draft.fatigue), true);
            // Derived stats and resistances.
            put(win(51.0, 282.0, 253.0, 18.0), &label(7, "Derived Stats"), EditorText::Derived(99), UiFont::Small, SCREEN_WHITE, true);
            let values = derived.by_stat_id();
            for (i, &y) in DERIVED_ROWS.iter().enumerate() {
                for (x, slot) in [(49.0, i), (178.0, i + 4)] {
                    let text = format!("{}: {}", stat_name(8 + slot as u32), values[slot]);
                    put(win(x, y, 125.0, 14.0), &text, EditorText::Derived(slot), UiFont::Body, SCREEN_WHITE, false);
                }
            }
            put(win(315.0, 282.0, 136.0, 18.0), &label(8, "Resistance"), EditorText::Resist(99), UiFont::Small, SCREEN_WHITE, true);
            for (i, &y) in RESIST_ROWS.iter().enumerate() {
                let text = format!("{}: {}", label(12 + i as u32, "?"), resist[i]);
                put(win(322.0, y, 105.0, 14.0), &text, EditorText::Resist(i), UiFont::Body, SCREEN_WHITE, false);
            }
            // The shown skill group: name (and training rank) and gauge per row.
            for row in 0..SKILL_GROUP_SIZE {
                let skill = group * SKILL_GROUP_SIZE + row;
                let dy = SKILL_ROW * row as f32;
                let name = rules.skills.skills.get(skill).map_or("", |k| k.name.as_str());
                let rank = c.training[skill];
                let text = match rank {
                    1..=3 => format!("{name} ({})", rules.skills.ranks[usize::from(rank) - 1]),
                    _ => name.to_string(),
                };
                put(win(520.0, 126.0 + dy, 200.0, 19.0), &text, EditorText::SkillName(skill), UiFont::Small, SCREEN_WHITE, false);
            }
            for row in 0..SKILL_GROUP_SIZE {
                let skill = group * SKILL_GROUP_SIZE + row;
                let dy = SKILL_ROW * row as f32;
                let width = gauge_width(rules.skill_level(c, skill));
                let fill = ImageNode { rect: Some(Rect::new(0.0, 0.0, width as f32, GAUGE_HEIGHT)), ..ImageNode::new(art.gauge.clone()) };
                let mut gauge = p.spawn((SkillGauge { skill, width }, fill, at(win(562.0, 150.0 + dy, width as f32, GAUGE_HEIGHT))));
                if width == 0 {
                    gauge.insert(Visibility::Hidden);
                }
            }
        })
        .id();
    commands.entity(root).add_child(content);
}

#[allow(clippy::too_many_arguments)]
fn on_activate(
    activate: On<Activate>,
    actions: Query<&EditorAction>,
    mut commands: Commands,
    mut next_state: ResMut<NextState<AppState>>,
    mut host: ResMut<CharacterHost>,
    data: Option<Res<CreationData>>,
    mut message: ResMut<HelpMessage>,
    sounds: Option<Res<MenuSounds>>,
    state: Res<State<AppState>>,
    (mut sheet, mut group): (ResMut<SheetHost>, ResMut<ShownSkillGroup>),
) {
    let Ok(&action) = actions.get(activate.entity) else { return };
    if let Some(sounds) = &sounds {
        sounds.play_button_press(&mut commands);
    }
    let rules = data.as_ref().map(|d| d.rules.clone()).unwrap_or_default();
    let change = |d: i32, raise, lower| if d > 0 { raise } else { lower };
    if *state.get() == AppState::InGame {
        let request = match action {
            EditorAction::Stat(i, d) if i < PRIMARY_STATS => change(d, CharacterAction::RaiseStat(i), CharacterAction::LowerStat(i)),
            EditorAction::HitPoints(d) => CharacterAction::AdjustPool(Pool::HitPoints, d),
            EditorAction::Fatigue(d) => CharacterAction::AdjustPool(Pool::Fatigue, d),
            EditorAction::SkillRow(row, d) => skill_change(group.0, row, d),
            EditorAction::SkillGroup(g) => {
                group.0 = g;
                return;
            }
            _ => return,
        };
        let result = sheet.request(&rules, SheetRequest::Action(request));
        message.0 = result.err().map(|e| refusal_text(e, data.as_deref()));
        return;
    }
    let result = match action {
        EditorAction::Back => {
            next_state.set(AppState::CreateCharacter);
            Ok(())
        }
        EditorAction::Next => host.finish(&rules).map(|_| {
            commands.insert_resource(LoadRequest::new(AppState::InGame).after_world_boots());
            next_state.set(AppState::Loading);
        }),
        EditorAction::Stat(i, d) if i < PRIMARY_STATS => {
            host.apply(&rules, if d > 0 { CharacterAction::RaiseStat(i) } else { CharacterAction::LowerStat(i) })
        }
        EditorAction::Stat(..) => Ok(()),
        EditorAction::HitPoints(d) => host.apply(&rules, CharacterAction::AdjustPool(Pool::HitPoints, d)),
        EditorAction::Fatigue(d) => host.apply(&rules, CharacterAction::AdjustPool(Pool::Fatigue, d)),
        EditorAction::SkillRow(row, d) => host.apply(&rules, skill_change(group.0, row, d)),
        EditorAction::SkillGroup(g) => {
            group.0 = g;
            Ok(())
        }
    };
    message.0 = result.err().map(|e| refusal_text(e, data.as_deref()));
}

/// The action for a skill row's button in the shown group.
fn skill_change(group: usize, row: usize, d: i32) -> CharacterAction {
    let skill = group * SKILL_GROUP_SIZE + row;
    if d > 0 { CharacterAction::RaiseSkill(skill) } else { CharacterAction::LowerSkill(skill) }
}

/// The original's message for a refused change.
fn refusal_text(e: arcanum_rules::Refusal, data: Option<&CreationData>) -> String {
    let text = match (e.message(), data) {
        (Some(Message::Charedit(id)), Some(d)) => d.charedit.get(id),
        (Some(Message::MainMenu(id)), Some(d)) => d.menu.get(id),
        _ => None,
    };
    text.unwrap_or("That cannot be changed.").to_string()
}
