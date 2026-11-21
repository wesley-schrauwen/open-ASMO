//! The message box in the middle of the in-game bottom bar (Arcanum.exe 0x5529c0; posting
//! 0x550750, clearing 0x550720).
//!
//! - The box is (211, 503, 383, 82) on screen, cleared to black. An icon for the kind of message
//!   sits at (217, 510) (table 0x5c708c: `Exclaimation_Icon` for refusals, the Arcanum icon for
//!   help, the college's icon for spells and colleges).
//! - With a title: the title (morph15font, white) in (290, 504, 291, 19) and the body
//!   (Flare12Font, white, wrapped) in (290, 528, 291, 54). Without one: the body in
//!   (290, 504, 291, 78). Skill and degree help add a right-aligned blue line in the title's rect
//!   ("Minimum ST to raise: 11"); spell help a line under the title ("Minimum WP: 6") and the
//!   description at (290, 546, 291, 36).
//! - Stored messages (refusals, and later level-ups and the like) replace what is shown. Hover
//!   help is shown only when no stored message came in the last 3 s of real time; leaving the
//!   hovered element clears the box, whatever it shows. Nothing expires by itself.
//! - Hover help: the bars' buttons from `mes/intgame.mes` 1000-1007; the character screen's
//!   elements from `mes/charedit.mes` 100-140 ("title\nbody") and the rules tables (skills
//!   1000+, spells 2000+, colleges 3000+, disciplines 4000+, degrees 5000+; 0x55ae70).
//!
//! Not built: the original's shortening when a text does not fit (it redraws without the icon,
//! 0x5c7148), the message log behind it (a 10-entry ring, 0x64c540), the clock's time and date,
//! the hotkey slots' and vials' help, college descriptions (not found), spells' fatigue line,
//! hover help in character creation (the original draws the box in the editor's own window).

use bevy::picking::Pickable;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;

use arcanum_rules::{CharacterSheet, LEVELS_PER_POINT, PRIMARY_STATS, SKILL_STAT, SPELLS_PER_COLLEGE, degree_intelligence, skill_cap, skill_points};

use crate::art_loader::ArtFrames;
use crate::create_character::CreationData;
use crate::fonts::{ArcanumFonts, UiFont};
use crate::main_menu::AppState;
use crate::menu_window::{SCREEN_WHITE, at};
use crate::resource_pack::paths::interface;

/// The box, on the 800x600 screen.
pub const BOX: Rect = Rect { min: Vec2::new(211.0, 503.0), max: Vec2::new(594.0, 585.0) };
const ICON_AT: Vec2 = Vec2::new(217.0, 510.0);
const TITLE: Rect = Rect { min: Vec2::new(290.0, 504.0), max: Vec2::new(581.0, 523.0) };
const SECOND_LINE: Rect = Rect { min: Vec2::new(290.0, 528.0), max: Vec2::new(581.0, 546.0) };
const BODY_UNDER_TITLE: Rect = Rect { min: Vec2::new(290.0, 528.0), max: Vec2::new(581.0, 582.0) };
const BODY_ALONE: Rect = Rect { min: Vec2::new(290.0, 504.0), max: Vec2::new(581.0, 582.0) };
const BODY_LOW: Rect = Rect { min: Vec2::new(290.0, 546.0), max: Vec2::new(581.0, 582.0) };
/// Hover help waits this long after a stored message (real seconds).
pub const HELP_HOLD: f64 = 3.0;
/// The requirement line on skill and degree help (0x64c49c: Flare12Font in pure blue).
pub const REQUIREMENT: Color = Color::srgb_u8(0, 0, 255);

/// The icon beside a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxIcon {
    /// `LevelUpIcon` (art 444, "Arcanum Icon"): help.
    Arcanum,
    /// `Exclaimation_Icon` (art 442): plain messages such as refusals.
    Exclamation,
    /// A college's icon (`s_<college>.art`): spells and colleges.
    College(usize),
}

/// Where a message's extra line goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtraLine {
    /// Right-aligned in blue in the title's rect (skills, degrees).
    Requirement(String),
    /// Under the title, with the body below it (spells).
    UnderTitle(String),
}

/// One message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxMessage {
    pub icon: BoxIcon,
    pub title: Option<String>,
    pub body: String,
    pub extra: Option<ExtraLine>,
}

impl BoxMessage {
    /// A plain stored message, such as a refusal (kind 4).
    pub fn plain(text: impl Into<String>) -> Self {
        Self { icon: BoxIcon::Exclamation, title: None, body: text.into(), extra: None }
    }

    /// Help text: a first line becomes the title, the rest the body (kinds 6 and 12).
    pub fn help(text: &str) -> Self {
        let (title, body) = match text.split_once('\n') {
            Some((t, b)) => (Some(t.trim().to_string()), b.trim().to_string()),
            None => (None, text.trim().to_string()),
        };
        Self { icon: BoxIcon::Arcanum, title, body, extra: None }
    }
}

/// What the box shows.
#[derive(Resource, Debug, Clone, Default)]
pub struct MessageBox {
    shown: Option<BoxMessage>,
    /// When the last stored message came (real seconds).
    last_stored: Option<f64>,
}

impl MessageBox {
    pub fn shown(&self) -> Option<&BoxMessage> {
        self.shown.as_ref()
    }

    /// Shows a stored message at once (it holds hover help back for [`HELP_HOLD`]).
    pub fn post(&mut self, message: BoxMessage, now: f64) {
        self.shown = Some(message);
        self.last_stored = Some(now);
    }

    /// Shows hover help unless a stored message came less than [`HELP_HOLD`] ago.
    pub fn show_help(&mut self, message: BoxMessage, now: f64) -> bool {
        if self.last_stored.is_some_and(|t| now - t < HELP_HOLD) {
            return false;
        }
        if self.shown.as_ref() != Some(&message) {
            self.shown = Some(message);
        }
        true
    }

    pub fn clear(&mut self) {
        if self.shown.is_some() {
            self.shown = None;
        }
    }
}

/// Hover help for an element: an `intgame.mes` id (the bars) or a character screen id.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxHelp {
    Intgame(u32),
    Editor(u32),
}

/// The box's root, in the bottom bar.
#[derive(Component)]
pub struct MessageBoxRoot;

/// Marks the box's texts, for tests.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxText {
    Title,
    Body,
    Extra,
}

#[derive(Resource)]
struct BoxArt {
    arcanum: Handle<Image>,
    exclamation: Handle<Image>,
    colleges: Vec<Handle<ArtFrames>>,
}

pub struct MessageBoxPlugin;

impl Plugin for MessageBoxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MessageBox>()
            .add_systems(Startup, |mut commands: Commands, assets: Res<AssetServer>| {
                commands.insert_resource(BoxArt {
                    arcanum: assets.load(interface::ARCANUM_ICON),
                    exclamation: assets.load(interface::EXCLAMATION_ICON),
                    colleges: crate::editor_tabs::college_buttons().into_iter().map(|p| assets.load(p)).collect(),
                });
            })
            .add_systems(OnExit(AppState::InGame), |mut b: ResMut<MessageBox>| *b = MessageBox::default())
            .add_systems(Update, (hover_help, show_box).chain().run_if(in_state(AppState::InGame)));
    }
}

/// A hover area that lets clicks and hovers through to what is under it.
pub fn help_area(help: BoxHelp, r: Rect) -> impl Bundle {
    (help, Hovered::default(), Pickable { should_block_lower: false, is_hoverable: true }, at(r))
}

/// Spawns the box into the bottom bar (`local` turns screen rects into the bar's).
pub fn spawn_box(bottom: &mut ChildSpawnerCommands, local: impl Fn(Rect) -> Rect) {
    bottom.spawn((MessageBoxRoot, Pickable::IGNORE, BackgroundColor(Color::BLACK), at(local(BOX))));
}

fn in_box(r: Rect) -> Rect {
    Rect { min: r.min - BOX.min, max: r.max - BOX.min }
}

/// Redraws the box when its message changes.
fn show_box(mut commands: Commands, messages: Res<MessageBox>, roots: Query<(Entity, Option<&Children>), With<MessageBoxRoot>>, art: Option<Res<BoxArt>>, frames: Option<Res<Assets<ArtFrames>>>, fonts: Option<Res<ArcanumFonts>>, added: Query<(), Added<MessageBoxRoot>>, mut pending: Local<bool>) {
    if !messages.is_changed() && added.is_empty() && !*pending {
        return;
    }
    *pending = false;
    // Loaded at startup; a test app may add the plugin later.
    let Some(art) = art else { return };
    let fonts = fonts.map(|f| f.clone()).unwrap_or_default();
    for (root, children) in &roots {
        for &child in children.into_iter().flatten() {
            commands.entity(child).despawn();
        }
        let Some(m) = messages.shown() else { continue };
        let icon = match m.icon {
            BoxIcon::Arcanum => Some(art.arcanum.clone()),
            BoxIcon::Exclamation => Some(art.exclamation.clone()),
            BoxIcon::College(c) => {
                let image = art.colleges.get(c).and_then(|h| frames.as_ref()?.get(h)).and_then(|f| f.frame(0)).cloned();
                // Not loaded yet: draw again next frame.
                *pending |= image.is_none();
                image
            }
        };
        commands.entity(root).with_children(|b| {
            if let Some(icon) = icon {
                b.spawn((Pickable::IGNORE, ImageNode::new(icon), Node { position_type: PositionType::Absolute, left: crate::menu_window::px(ICON_AT.x - BOX.min.x), top: crate::menu_window::px(ICON_AT.y - BOX.min.y), ..default() }));
            }
            let mut line = |r: Rect, text: &str, which: BoxText, font: UiFont, colour: Color, justify: JustifyContent| {
                b.spawn((Pickable::IGNORE, Node { justify_content: justify, align_items: AlignItems::FlexStart, overflow: Overflow::clip(), ..at(in_box(r)) })).with_children(|cell| {
                    let mut t = cell.spawn((which, TextColor(colour)));
                    fonts.insert_text(&mut t, text, font);
                });
            };
            let body_at = match (&m.title, &m.extra) {
                (None, _) => BODY_ALONE,
                (Some(_), Some(ExtraLine::UnderTitle(_))) => BODY_LOW,
                (Some(_), _) => BODY_UNDER_TITLE,
            };
            if let Some(title) = &m.title {
                line(TITLE, title, BoxText::Title, UiFont::Small, SCREEN_WHITE, JustifyContent::FlexStart);
            }
            match &m.extra {
                Some(ExtraLine::Requirement(text)) => line(TITLE, text, BoxText::Extra, UiFont::Body, REQUIREMENT, JustifyContent::FlexEnd),
                Some(ExtraLine::UnderTitle(text)) => line(SECOND_LINE, text, BoxText::Extra, UiFont::Body, SCREEN_WHITE, JustifyContent::FlexStart),
                None => {}
            }
            b.spawn((Pickable::IGNORE, Node { overflow: Overflow::clip(), ..at(in_box(body_at)) })).with_children(|cell| {
                let mut t = cell.spawn((BoxText::Body, TextColor(SCREEN_WHITE)));
                let scale = crate::fonts::ORIGINAL_SCALE;
                fonts.insert_block(&mut t, &m.body, UiFont::Body, body_at.width() * scale, Some(body_at.height() * scale), crate::bitmap_text::Align::Left);
            });
        });
    }
}

/// Shows the hovered element's help; leaving it clears the box (0x54dc80, 0x55a718).
#[allow(clippy::too_many_arguments)]
fn hover_help(
    helps: Query<(&BoxHelp, &Hovered)>,
    data: Option<Res<CreationData>>,
    sheet: Option<Res<crate::pc_sheet::SheetHost>>,
    mut messages: ResMut<MessageBox>,
    time: Option<Res<Time<Real>>>,
    mut last: Local<Option<BoxHelp>>,
) {
    let hovered = hovered_help(&helps);
    if hovered != *last {
        if last.is_some() {
            messages.clear();
        }
        *last = hovered;
    }
    let (Some(help), Some(data)) = (hovered, data) else { return };
    let message = match help {
        BoxHelp::Intgame(id) => data.intgame.get(id).map(BoxMessage::help),
        BoxHelp::Editor(id) => sheet.as_ref().and_then(|s| s.sheet()).and_then(|c| editor_help(id, &data, c)),
    };
    if let Some(message) = message {
        messages.show_help(message, time.map_or(0.0, |t| t.elapsed_secs_f64()));
    }
}

/// The hovered help; where hover areas overlap (the info panel's 101 holds 103-105), the
/// highest id wins: the more specific one.
fn hovered_help(helps: &Query<(&BoxHelp, &Hovered)>) -> Option<BoxHelp> {
    let id = |h: &BoxHelp| match *h {
        BoxHelp::Intgame(i) | BoxHelp::Editor(i) => i,
    };
    helps.iter().filter(|(_, h)| h.get()).map(|(help, _)| *help).max_by_key(id)
}

/// The lowest governing stat that allows skill `k` another point, if it has room for one.
pub fn stat_to_raise(k: usize, sheet: &CharacterSheet) -> Option<i32> {
    let wanted = (skill_points(sheet, k) + 1) * LEVELS_PER_POINT;
    (1..=24).find(|&s| skill_cap(s) >= wanted)
}

/// The character screen's help for `id` (0x55ae70).
pub fn editor_help(id: u32, data: &CreationData, sheet: &CharacterSheet) -> Option<BoxMessage> {
    let rules = &data.rules;
    let idx = |base: u32| (id - base) as usize;
    let arcanum = |title: &str, body: &str, extra| BoxMessage { icon: BoxIcon::Arcanum, title: Some(title.to_string()), body: body.to_string(), extra };
    match id {
        1000..=1999 => {
            let k = idx(1000);
            let skill = rules.skills.skills.get(k)?;
            let stat = SKILL_STAT[k];
            let extra = stat_to_raise(k, sheet).filter(|_| stat < PRIMARY_STATS).map(|need| {
                let label = data.intgame.get(28 + stat as u32).unwrap_or("Minimum to raise");
                ExtraLine::Requirement(format!("{label}: {need}"))
            });
            Some(arcanum(&skill.name, &skill.description, extra))
        }
        2000..=2999 => {
            let spell = idx(2000);
            let s = rules.spells.spells.get(spell)?;
            let rank = spell % SPELLS_PER_COLLEGE;
            let wp = [6, 9, 12, 15, 18][rank];
            let label = data.intgame.get(73).unwrap_or("Minimum WP");
            Some(BoxMessage { icon: BoxIcon::College(s.college), extra: Some(ExtraLine::UnderTitle(format!("{label}: {wp}"))), ..arcanum(&s.name, &s.description, None) })
        }
        3000..=3999 => {
            let c = idx(3000);
            let name = rules.spells.colleges.get(c)?;
            Some(BoxMessage { icon: BoxIcon::College(c), ..arcanum(name, "", None) })
        }
        4000..=4999 => {
            let d = rules.tech.disciplines.get(idx(4000))?;
            Some(arcanum(&d.name, &d.description, None))
        }
        5000..=5999 => {
            let (d, degree) = (idx(5000) / 8, idx(5000) % 8);
            let discipline = rules.tech.disciplines.get(d)?;
            let label = data.intgame.get(32).unwrap_or("Minimum IN to raise");
            let extra = (degree > 0).then(|| ExtraLine::Requirement(format!("{label}: {}", degree_intelligence(degree as i32))));
            Some(arcanum(&rules.tech.degrees[degree], &discipline.degree_descriptions[degree], extra))
        }
        _ => data.charedit.get(id).map(BoxMessage::help),
    }
}

/// The box's text rects, for tests.
pub fn rects() -> [Rect; 5] {
    [TITLE, SECOND_LINE, BODY_UNDER_TITLE, BODY_ALONE, BODY_LOW]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_help_waits_three_seconds_after_a_stored_message() {
        let mut b = MessageBox::default();
        assert!(b.show_help(BoxMessage::help("Skills"), 0.0));
        b.post(BoxMessage::plain("You haven't enough character points."), 10.0);
        assert!(!b.show_help(BoxMessage::help("Skills"), 12.9));
        assert_eq!(b.shown().unwrap().body, "You haven't enough character points.");
        assert!(b.show_help(BoxMessage::help("Skills"), 13.0));
        b.clear();
        assert!(b.shown().is_none());
    }

    #[test]
    fn help_texts_split_into_title_and_body() {
        let m = BoxMessage::help("Strength\nHow strong you are.");
        assert_eq!((m.title.as_deref(), m.body.as_str()), (Some("Strength"), "How strong you are."));
        assert_eq!(BoxMessage::help("Inventory").title, None);
    }

    #[test]
    fn skill_help_names_the_stat_the_next_point_needs() {
        let s = arcanum_rules::RulesData::default().new_sheet();
        assert_eq!(stat_to_raise(0, &s), Some(6), "the first point needs a cap of 4: stats 1-5 give 3");
    }

    #[test]
    fn the_character_screen_help_comes_from_the_rules_tables() {
        use arcanum_formats::mes::MesFile;
        let mes = |t: &str| MesFile::parse(t.as_bytes()).unwrap();
        let mut data = CreationData { intgame: mes("{29}{Minimum DX to raise}{32}{Minimum IN to raise}{73}{Minimum WP}"), charedit: mes("{109}{Strength\nHow strong.}"), ..Default::default() };
        data.rules.skills = arcanum_rules::SkillTable::from_mes(&mes("{0}{Bow}{20}{Bows. (DX)}"));
        data.rules.tech = arcanum_rules::TechTable::from_mes(&mes("{0}{Herbology}{8}{Layman}{9}{Novice}{25}{Salves.}"));
        data.rules.spells = arcanum_rules::SpellTable::from_mes(&mes("{7}{Read Aura}{700}{x}{707}{Auras.}"));
        let s = data.rules.new_sheet();
        let bow = editor_help(1000, &data, &s).unwrap();
        assert_eq!((bow.title.as_deref(), bow.body.as_str()), (Some("Bow"), "Bows. (DX)"));
        assert_eq!(bow.extra, Some(ExtraLine::Requirement("Minimum DX to raise: 6".into())));
        let novice = editor_help(5001, &data, &s).unwrap();
        assert_eq!((novice.title.as_deref(), novice.body.as_str()), (Some("Novice"), "Salves."));
        assert_eq!(novice.extra, Some(ExtraLine::Requirement("Minimum IN to raise: 5".into())));
        let aura = editor_help(2007, &data, &s).unwrap();
        assert_eq!((aura.icon, aura.extra), (BoxIcon::College(1), Some(ExtraLine::UnderTitle("Minimum WP: 12".into()))));
        assert_eq!(editor_help(109, &data, &s).unwrap().title.as_deref(), Some("Strength"));
    }

    #[test]
    fn the_text_rects_sit_inside_the_box() {
        for r in rects() {
            assert!(BOX.contains(r.min) && BOX.contains(r.max), "{r:?}");
        }
    }
}
