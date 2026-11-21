//! The character editor's tabs on the right: skills, tech and spells (Arcanum.exe 0x55b150
//! dispatches on the tab, 0x55bd10 shows one child window and hides the others). Positions are
//! the original's editor coordinates (screen y = editor y + 41); each tab's window is 235x325 at
//! (503, 63). The skills tab is shown first.
//!
//! - Tabs (art 34-36, 39x39) at (527/595/663, 11); the shown one keeps its pressed frame (the
//!   original's marking was not found).
//! - **Skills** (0x55bab0, 0x55bf20): `Skills_Window` (art 29); group buttons (art 302-305) at
//!   x 524/578/632/688, y 80; four rows 66 apart: the name (and training rank, " (Expert)") at
//!   (520, 126) in morph15font, a gauge (`SkilGauge`, art 623) at (562, 150) filled
//!   `120 x level / 20` wide, minus/plus (art 625/624) at x 521/695, y 147. No numbers.
//! - **Tech** (0x55c110, 0x55c3a0): `Disciplines_Window` (art 30); eight discipline buttons
//!   (art 306-313, 32x31) in a zigzag from (511, 70), Herbology first. A header: the
//!   discipline's glyph (Icons17Font, `'1' + discipline`) at (520, 133), "<discipline>
//!   <degree>" centred on x 621 (Nick16Font) and the degree centred on x 707 (Flare12Font).
//!   Degrees 1-7 in rows 33 apart from y 157: the schematic the degree teaches, white when
//!   learned, cyan for the next, grey after. Each learned degree shows the discipline's icon
//!   (`TTI_*`, art 649-656) at (511, 159 + 33 x row); the plus button (art 647) sits on the next
//!   row's slot and the minus (art 648) on the last bought degree.
//! - **Spells** (0x55c890, 0x55cbc0): `char_spells` (art 31); sixteen college buttons (24x24) in
//!   two rows of eight at y 78 and 103, Conveyance first; five spell rows 47-48 apart: the
//!   spell's icon (32x32) at (521, 145), its name at (562, 139) and "Level: n" at (562, 160),
//!   white when known, cyan for the next, grey after; plus on the next spell's icon, minus on
//!   the last bought one. Unknown spells' icons are dimmed (the original's palette effect was
//!   not decoded).
//!
//! Not built: the ingredients line under each degree, the hover help (charedit 132-139, tech
//! 4000/5000, spells 2000/3000), the fourth tab (art 568).

use bevy::picking::Pickable;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{ActivateOnPress, Button};

use arcanum_rules::{
    CharacterSheet, COLLEGES, DISCIPLINES, MAX_DEGREE, MAX_SKILL_LEVEL, SKILL_GROUP_SIZE, SPELLS_PER_COLLEGE, SchematicSource, spells_known, tech_degree,
};

use crate::art_loader::ArtFrames;
use crate::bitmap_text::{BitmapFontAsset, BitmapText};
use crate::character_editor::EditorAction;
use crate::create_character::CreationData;
use crate::fonts::{ArcanumFonts, ORIGINAL_SCALE, UiFont};
use crate::menu_window::{SCREEN_WHITE, TOP_BAR_HEIGHT, at, rect};
use crate::resource_pack::paths::interface;
use crate::widgets::FrameButton;

pub const SKILLS_TAB: usize = 0;
pub const TECH_TAB: usize = 1;
pub const SPELLS_TAB: usize = 2;

/// What the tabs show: the tab, and the skill group, discipline and college picked in each.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EditorView {
    pub tab: usize,
    pub group: usize,
    pub discipline: usize,
    pub college: usize,
}

/// A tab's window and buttons, shown only while its tab is.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabPage(pub usize);

/// Marks the tabs' texts, for tests.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabText {
    /// A skill's name row (`mes/skill.mes` index).
    SkillName(usize),
    TechTitle,
    TechDegree,
    /// A degree row (1-7) of the shown discipline.
    DegreeRow(usize),
    /// A spell row (0-4) of the shown college.
    SpellName(usize),
    SpellLevel(usize),
}

/// A skill's gauge, with its fill width in the original's pixels.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillGauge {
    pub skill: usize,
    pub width: u32,
}

/// A spell icon on the spells tab (index into `mes/spell.mes`), and whether it is known.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpellIcon {
    pub spell: usize,
    pub known: bool,
}

/// The next degree or spell, which can be bought (0x55c3a0: cyan, `(0, 210, 255)`).
pub const NEXT: Color = Color::srgb_u8(0, 210, 255);
/// Degrees and spells beyond the next.
pub const LATER: Color = Color::srgb_u8(128, 128, 128);

const TAB_X: [f32; 3] = [527.0, 595.0, 663.0];
const WINDOW: Vec2 = Vec2::new(503.0, 63.0);
const WINDOW_SIZE: Vec2 = Vec2::new(235.0, 325.0);
const GROUP_X: [f32; 4] = [524.0, 578.0, 632.0, 688.0];
const GROUP_SIZE: [(f32, f32); 4] = [(32.0, 32.0), (33.0, 32.0), (34.0, 32.0), (32.0, 32.0)];
const SKILL_ROW: f32 = 66.0;
const GAUGE_WIDTH: f32 = 120.0;
const GAUGE_HEIGHT: f32 = 27.0;
/// Discipline buttons, window coordinates (0x5c87d8).
const DISCIPLINE_AT: [(f32, f32); DISCIPLINES] = [(8.0, 7.0), (34.0, 29.0), (61.0, 7.0), (88.0, 28.0), (116.0, 7.0), (142.0, 29.0), (171.0, 7.0), (198.0, 29.0)];
const DEGREE_ROW: f32 = 33.0;
/// College buttons (0x5c8630): two rows of eight.
const COLLEGE_X: [[f32; 8]; 2] = [[516.0, 541.0, 566.0, 591.0, 616.0, 641.0, 665.0, 690.0], [528.0, 553.0, 578.0, 603.0, 627.0, 652.0, 677.0, 702.0]];
const COLLEGE_Y: [f32; 2] = [78.0, 103.0];
const SPELL_ICON_Y: [f32; SPELLS_PER_COLLEGE] = [145.0, 193.0, 240.0, 288.0, 335.0];
const SPELL_NAME_Y: [f32; SPELLS_PER_COLLEGE] = [139.0, 187.0, 234.0, 282.0, 329.0];
const SPELL_LEVEL_Y: [f32; SPELLS_PER_COLLEGE] = [160.0, 208.0, 255.0, 303.0, 350.0];
/// Level each spell rank needs (shown as "Level: n").
const SPELL_LEVEL: [i32; SPELLS_PER_COLLEGE] = [1, 1, 5, 10, 15];

/// The gauge's fill width for a skill level: `120 x level / 20`, rounded down (0x55bf20).
pub fn gauge_width(level: i32) -> u32 {
    (GAUGE_WIDTH as i32 * level.clamp(0, MAX_SKILL_LEVEL) / MAX_SKILL_LEVEL) as u32
}

/// Editor coordinates to the 800x600 screen.
fn win(x: f32, y: f32, w: f32, h: f32) -> Rect {
    rect(Vec2::new(x, y + TOP_BAR_HEIGHT), Vec2::new(w, h))
}

/// The art stems of the colleges, in `mes/spell.mes` order: `s_<stem>.art` is the college's
/// button, `s_<stem>1.art` to `5` its spells' icons (force's button is `s_forc.art`).
const COLLEGE_STEMS: [&str; COLLEGES] =
    ["conveyance", "divination", "air", "earth", "fire", "water", "force", "mental", "meta", "morph", "nature", "evilnecro", "goodnecro", "phantasm", "summoning", "temporal"];
const DISCIPLINE_BUTTONS: [&str; DISCIPLINES] = ["Anatomical", "Chemistry", "Electrical", "Explosives", "GunSmithy", "Mechanical", "Smithy", "Therapeutics"];
const DISCIPLINE_ICONS: [&str; DISCIPLINES] = ["Herb", "Chem", "Electric", "Explosives", "Gun", "Mechanical", "Smithy", "Theru"];

#[derive(Resource)]
pub struct TabArt {
    tabs: [Handle<ArtFrames>; 3],
    skills_window: Handle<Image>,
    groups: [Handle<ArtFrames>; 4],
    gauge: Handle<Image>,
    skill_plus: Handle<ArtFrames>,
    skill_minus: Handle<ArtFrames>,
    disciplines_window: Handle<Image>,
    disciplines: [Handle<ArtFrames>; DISCIPLINES],
    degree_icons: [Handle<Image>; DISCIPLINES],
    add: Handle<ArtFrames>,
    remove: Handle<ArtFrames>,
    spells_window: Handle<Image>,
    colleges: [Handle<ArtFrames>; COLLEGES],
    spell_icons: Vec<Handle<Image>>,
    glyphs: Handle<BitmapFontAsset>,
    title: Handle<BitmapFontAsset>,
}

impl TabArt {
    pub fn load(assets: &AssetServer) -> Self {
        let dir = interface::DIR;
        Self {
            tabs: interface::CHAR_TABS.map(|p| assets.load(p)),
            skills_window: assets.load(interface::SKILLS_WINDOW),
            groups: interface::SKILL_GROUPS.map(|p| assets.load(p)),
            gauge: assets.load(interface::SKILL_GAUGE),
            skill_plus: assets.load(interface::SKILL_PLUS),
            skill_minus: assets.load(interface::SKILL_MINUS),
            disciplines_window: assets.load(interface::DISCIPLINES_WINDOW),
            disciplines: DISCIPLINE_BUTTONS.map(|n| assets.load(format!("{dir}/{n}_But.ART"))),
            degree_icons: DISCIPLINE_ICONS.map(|n| assets.load(format!("{dir}/TTI_{n}.ART"))),
            add: assets.load(interface::SPELL_TECH_ADD),
            remove: assets.load(interface::SPELL_TECH_MINUS),
            spells_window: assets.load(interface::SPELLS_WINDOW),
            colleges: COLLEGE_STEMS.map(|s| assets.load(format!("{dir}/s_{}.art", if s == "force" { "forc" } else { s }))),
            spell_icons: COLLEGE_STEMS.iter().flat_map(|s| (1..=SPELLS_PER_COLLEGE).map(move |i| format!("{dir}/s_{s}{i}.art"))).map(|p| assets.load(p)).collect(),
            glyphs: assets.load(crate::resource_pack::paths::fonts::ICONS),
            title: assets.load(crate::resource_pack::paths::fonts::TITLE),
        }
    }
}

fn button(action: EditorAction, frames: &Handle<ArtFrames>, r: Rect) -> impl Bundle {
    (action, Button, ActivateOnPress, Hovered::default(), FrameButton::arrow(frames.clone()), ImageNode::default(), at(r))
}

fn page(tab: usize) -> impl Bundle {
    let shown = if tab == SKILLS_TAB { Visibility::Inherited } else { Visibility::Hidden };
    (TabPage(tab), shown, Pickable::IGNORE, Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() })
}

/// The tabs, and each tab's window and fixed buttons.
pub fn spawn_tabs(screen: &mut ChildSpawnerCommands, art: &TabArt) {
    for (t, (&x, frames)) in TAB_X.iter().zip(&art.tabs).enumerate() {
        screen.spawn(button(EditorAction::Tab(t), frames, win(x, 11.0, 39.0, 39.0)));
    }
    let window = |image: &Handle<Image>| (Pickable::IGNORE, ImageNode::new(image.clone()), at(win(WINDOW.x, WINDOW.y, WINDOW_SIZE.x, WINDOW_SIZE.y)));
    screen.spawn(page(SKILLS_TAB)).with_children(|p| {
        p.spawn(window(&art.skills_window));
        for (g, (&x, frames)) in GROUP_X.iter().zip(&art.groups).enumerate() {
            let (w, h) = GROUP_SIZE[g];
            p.spawn(button(EditorAction::SkillGroup(g), frames, win(x, 80.0, w, h)));
        }
        for row in 0..SKILL_GROUP_SIZE {
            let y = 147.0 + SKILL_ROW * row as f32;
            p.spawn(button(EditorAction::SkillRow(row, -1), &art.skill_minus, win(521.0, y, 29.0, 30.0)));
            p.spawn(button(EditorAction::SkillRow(row, 1), &art.skill_plus, win(695.0, y, 29.0, 30.0)));
        }
    });
    screen.spawn(page(TECH_TAB)).with_children(|p| {
        p.spawn(window(&art.disciplines_window));
        for (d, &(x, y)) in DISCIPLINE_AT.iter().enumerate() {
            p.spawn(button(EditorAction::Discipline(d), &art.disciplines[d], win(WINDOW.x + x, WINDOW.y + y, 32.0, 31.0)));
        }
    });
    screen.spawn(page(SPELLS_TAB)).with_children(|p| {
        p.spawn(window(&art.spells_window));
        for c in 0..COLLEGES {
            let (row, i) = (c / 8, c % 8);
            p.spawn(button(EditorAction::College(c), &art.colleges[c], win(COLLEGE_X[row][i], COLLEGE_Y[row], 24.0, 24.0)));
        }
    });
}

/// Shows the shown tab's page, and keeps the picked tab, group, discipline and college buttons
/// in their pressed frame.
pub fn show_view(view: Res<EditorView>, mut pages: Query<(&TabPage, &mut Visibility)>, mut buttons: Query<(&EditorAction, &mut FrameButton)>, added: Query<(), Added<EditorAction>>) {
    if !view.is_changed() && added.is_empty() {
        return;
    }
    for (page, mut shown) in &mut pages {
        let wanted = if page.0 == view.tab { Visibility::Inherited } else { Visibility::Hidden };
        shown.set_if_neq(wanted);
    }
    for (action, mut look) in &mut buttons {
        let picked = match *action {
            EditorAction::Tab(t) => t == view.tab,
            EditorAction::SkillGroup(g) => g == view.group,
            EditorAction::Discipline(d) => d == view.discipline,
            EditorAction::College(c) => c == view.college,
            _ => continue,
        };
        let normal = if picked { look.pressed } else { 0 };
        if look.normal != normal {
            look.normal = normal;
        }
    }
}

/// Draws the shown tab's values under `p`.
pub fn draw_tab(p: &mut ChildSpawnerCommands, c: &CharacterSheet, data: &CreationData, fonts: &ArcanumFonts, art: &TabArt, view: EditorView) {
    let rules = &data.rules;
    let text = |p: &mut ChildSpawnerCommands, r: Rect, s: &str, which: TabText, font: UiFont, colour: Color, centre: bool| {
        let justify = if centre { JustifyContent::Center } else { JustifyContent::FlexStart };
        p.spawn((Pickable::IGNORE, Node { justify_content: justify, align_items: AlignItems::Center, ..at(r) })).with_children(|cell| {
            let mut t = cell.spawn((which, TextColor(colour)));
            fonts.insert_text(&mut t, s, font);
        });
    };
    let bitmap = |p: &mut ChildSpawnerCommands, r: Rect, s: &str, which: TabText, font: &Handle<BitmapFontAsset>, centre: bool| {
        let justify = if centre { JustifyContent::Center } else { JustifyContent::FlexStart };
        p.spawn((Pickable::IGNORE, Node { justify_content: justify, align_items: AlignItems::Center, ..at(r) })).with_children(|cell| {
            cell.spawn((which, TextColor(SCREEN_WHITE), BitmapText::new(s, font.clone(), ORIGINAL_SCALE)));
        });
    };
    let colour = |n: i32, known: i32| match n {
        n if n < known => SCREEN_WHITE,
        n if n == known => NEXT,
        _ => LATER,
    };
    match view.tab {
        SKILLS_TAB => {
            for row in 0..SKILL_GROUP_SIZE {
                let skill = view.group * SKILL_GROUP_SIZE + row;
                let dy = SKILL_ROW * row as f32;
                let name = rules.skills.skills.get(skill).map_or("", |k| k.name.as_str());
                let label = match c.training[skill] {
                    rank @ 1..=3 => format!("{name} ({})", rules.skills.ranks[usize::from(rank) - 1]),
                    _ => name.to_string(),
                };
                text(p, win(520.0, 126.0 + dy, 200.0, 19.0), &label, TabText::SkillName(skill), UiFont::Small, SCREEN_WHITE, false);
                let width = gauge_width(rules.skill_level(c, skill));
                let fill = ImageNode { rect: Some(Rect::new(0.0, 0.0, width as f32, GAUGE_HEIGHT)), ..ImageNode::new(art.gauge.clone()) };
                let mut gauge = p.spawn((Pickable::IGNORE, SkillGauge { skill, width }, fill, at(win(562.0, 150.0 + dy, width as f32, GAUGE_HEIGHT))));
                if width == 0 {
                    gauge.insert(Visibility::Hidden);
                }
            }
        }
        TECH_TAB => {
            let d = view.discipline;
            let degree = tech_degree(c, d);
            let discipline = rules.tech.disciplines.get(d).map_or("", |x| x.name.as_str());
            let degree_name = &rules.tech.degrees[degree.clamp(0, MAX_DEGREE) as usize];
            bitmap(p, win(520.0, 133.0, 20.0, 19.0), &char::from(b'1' + d as u8).to_string(), TabText::TechTitle, &art.glyphs, false);
            bitmap(p, win(546.0, 132.0, 150.0, 19.0), &format!("{discipline} {degree_name}"), TabText::TechTitle, &art.title, true);
            text(p, win(692.0, 135.0, 30.0, 16.0), &degree.to_string(), TabText::TechDegree, UiFont::Body, SCREEN_WHITE, true);
            for k in 1..=MAX_DEGREE {
                let dy = DEGREE_ROW * (k - 1) as f32;
                let name = rules
                    .schematics
                    .iter()
                    .find(|s| s.source == SchematicSource::Learned { discipline: d, degree: k as usize })
                    .and_then(|s| data.schematic_text.get(s.name))
                    .unwrap_or("");
                text(p, win(549.0, 157.0 + dy, 170.0, 16.0), name, TabText::DegreeRow(k as usize), UiFont::Small, colour(k - 1, degree), false);
                if k <= degree {
                    p.spawn((Pickable::IGNORE, ImageNode::new(art.degree_icons[d].clone()), at(win(511.0, 159.0 + dy, 32.0, 32.0))));
                }
            }
            if degree < MAX_DEGREE {
                p.spawn(button(EditorAction::TechStep(1), &art.add, win(511.0, 159.0 + DEGREE_ROW * degree as f32, 32.0, 32.0)));
            }
            if c.draft.tech[d] > 0 {
                p.spawn(button(EditorAction::TechStep(-1), &art.remove, win(511.0, 159.0 + DEGREE_ROW * (degree - 1) as f32, 32.0, 32.0)));
            }
        }
        _ => {
            let college = view.college;
            let known = spells_known(c)[college];
            let level_label = data.charedit.get(1).unwrap_or("Level");
            for i in 0..SPELLS_PER_COLLEGE {
                let spell = college * SPELLS_PER_COLLEGE + i;
                let is_known = (i as i32) < known;
                let tint = if is_known { Color::WHITE } else { Color::srgb(0.45, 0.45, 0.45) };
                p.spawn((
                    Pickable::IGNORE,
                    SpellIcon { spell, known: is_known },
                    ImageNode { color: tint, ..ImageNode::new(art.spell_icons[spell].clone()) },
                    at(win(521.0, SPELL_ICON_Y[i], 32.0, 32.0)),
                ));
                let name = rules.spells.spells.get(spell).map_or("", |s| s.name.as_str());
                let shade = colour(i as i32, known);
                text(p, win(562.0, SPELL_NAME_Y[i], 160.0, 19.0), name, TabText::SpellName(i), UiFont::Small, shade, false);
                text(p, win(562.0, SPELL_LEVEL_Y[i], 160.0, 16.0), &format!("{level_label}: {}", SPELL_LEVEL[i]), TabText::SpellLevel(i), UiFont::Body, shade, false);
            }
            if (known as usize) < SPELLS_PER_COLLEGE {
                p.spawn(button(EditorAction::SpellStep(1), &art.add, win(521.0, SPELL_ICON_Y[known as usize], 32.0, 32.0)));
            }
            if c.draft.spells[college] > 0 {
                p.spawn(button(EditorAction::SpellStep(-1), &art.remove, win(521.0, SPELL_ICON_Y[known as usize - 1], 32.0, 32.0)));
            }
        }
    }
}

/// The rules action for a tab button, or `None` when it only changes the view.
pub fn tab_change(view: &mut EditorView, action: EditorAction) -> Option<arcanum_rules::CharacterAction> {
    use arcanum_rules::CharacterAction as A;
    let step = |d: i32, up: A, down: A| Some(if d > 0 { up } else { down });
    match action {
        EditorAction::Tab(t) => view.tab = t,
        EditorAction::SkillGroup(g) => view.group = g,
        EditorAction::Discipline(d) => view.discipline = d,
        EditorAction::College(c) => view.college = c,
        EditorAction::SkillRow(row, d) => {
            let skill = view.group * SKILL_GROUP_SIZE + row;
            return step(d, A::RaiseSkill(skill), A::LowerSkill(skill));
        }
        EditorAction::TechStep(d) => return step(d, A::RaiseTech(view.discipline), A::LowerTech(view.discipline)),
        EditorAction::SpellStep(d) => return step(d, A::RaiseSpell(view.college), A::LowerSpell(view.college)),
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauges_fill_six_pixels_a_level() {
        assert_eq!([0, 1, 4, 19, 20, 25].map(gauge_width), [0, 6, 24, 114, 120, 120]);
    }

    #[test]
    fn tab_buttons_pick_what_they_act_on() {
        use arcanum_rules::CharacterAction as A;
        let mut view = EditorView::default();
        assert_eq!(tab_change(&mut view, EditorAction::SkillGroup(2)), None);
        assert_eq!(tab_change(&mut view, EditorAction::SkillRow(3, 1)), Some(A::RaiseSkill(11)));
        tab_change(&mut view, EditorAction::Discipline(5));
        assert_eq!(tab_change(&mut view, EditorAction::TechStep(-1)), Some(A::LowerTech(5)));
        tab_change(&mut view, EditorAction::College(15));
        assert_eq!(tab_change(&mut view, EditorAction::SpellStep(1)), Some(A::RaiseSpell(15)));
        assert_eq!(view, EditorView { tab: 0, group: 2, discipline: 5, college: 15 });
    }
}
