//! The written window: books, notes, plaques and images a script shows (Arcanum.exe 0x56bc00
//! open, 0x56c050 draw, 0x56bfc0 input; the host sends [`arcanum_sim::LedgerDelta::Written`]).
//!
//! - **The window** (0x51cb90, rect 0x5ca480): (0, 41, 800, 399), under the top bar. Art 494
//!   is drawn over all of it, then the kind's background (table 0x5ca4b8) at (132, 0): book
//!   477, note 489, newspaper 487, telegram 491, plaque 633; an image is the interface art the
//!   script names.
//! - **Entries** (0x56c9f0): an entry starts with its font (an interface art number, a font
//!   `.ART`) and a justification letter (`c` centre, anything else left), then one space and
//!   the text. They stack down their rectangle, each wrapped to its width (0x56c800).
//! - **A book** (`mes/gamebook.mes`): two pages, (227, 57) and (476, 57), each 200x290; entries
//!   from the line, at most ten, up to the first missing one; an entry runs on over the pages.
//!   Previous (art 495 at (213, 77)) and next (496 at (675, 77)) turn two pages, shown only
//!   when there is somewhere to go.
//! - **A plaque** (`mes/gameplaque.mes`): the same flow in (269, 40, 428, 327), at most ten
//!   entries, in white (0x56c800 colour 0xff); the others in black (colour 0).
//! - **A note** (`mes/gamenote.mes`): the entries joined, in font 497, left, in (349, 49, 215, 300).
//! - **Closing:** a left click on the world (0x56bfc0 → 0x551000) closes it; opening another
//!   replaces it. Builder's calls: any click outside the page buttons closes it, and so does
//!   Escape; while it is open the world takes no clicks.
//!
//! Not built: newspapers and telegrams (their header tables 0x5ca638 / 0x5ca548 and columns
//! 0x5ca738); the alternate background 817 (0x56bb60); the sounds (3008 open, 3009 close, 3010
//! turn); text past what fits the last page (the original keeps it for the next spread, so does
//! this, up to 100 pages).

use arcanum_formats::font::BitmapFont;
use arcanum_formats::mes::MesFile;
use bevy::prelude::*;

use crate::bitmap_text::{Align, BitmapFontAsset, BitmapText};
use crate::fonts::ORIGINAL_SCALE;
use crate::main_menu::AppState;
use crate::menu_window::{at, rect};
use crate::resource_pack::paths::interface;
use crate::vfs_asset::VfsResource;

/// The window, in screen pixels (0x5ca480).
pub const WINDOW: Rect = Rect { min: Vec2::new(0.0, 41.0), max: Vec2::new(800.0, 440.0) };
/// Art drawn over the whole window first.
pub const BACKDROP_ART: u32 = 494;
/// Where the background goes, in the window.
pub const BACKGROUND_AT: Vec2 = Vec2::new(132.0, 0.0);
/// The backgrounds by kind (table 0x5ca4b8): book, note, newspaper, telegram, image, -, plaque.
pub const BACKGROUNDS: [u32; 7] = [477, 489, 487, 491, 0, 0, 633];
/// A book's pages (0x5ca618), in the window.
pub const BOOK_PAGES: [Rect; 2] = [Rect { min: Vec2::new(227.0, 57.0), max: Vec2::new(427.0, 347.0) }, Rect { min: Vec2::new(476.0, 57.0), max: Vec2::new(676.0, 347.0) }];
/// A plaque's text (0x5ca528) and a note's (0x5ca4f8).
pub const PLAQUE_TEXT: Rect = Rect { min: Vec2::new(269.0, 40.0), max: Vec2::new(697.0, 367.0) };
pub const NOTE_TEXT: Rect = Rect { min: Vec2::new(349.0, 49.0), max: Vec2::new(564.0, 349.0) };
/// A note's font.
pub const NOTE_FONT: u32 = 497;
/// The page buttons (0x5ca4d8): previous and next, art and corner in the window.
pub const PREVIOUS: (u32, Vec2) = (495, Vec2::new(213.0, 77.0));
pub const NEXT: (u32, Vec2) = (496, Vec2::new(675.0, 77.0));
/// Most entries a book or plaque shows from its line.
pub const MOST_ENTRIES: u32 = 10;
/// Most pages a book keeps (0x680dd4 below 100).
pub const MOST_PAGES: usize = 100;

/// The open window: kind, line, and the left page shown.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct WrittenPanel {
    pub open: Option<(u8, i32)>,
    pub page: usize,
}

impl WrittenPanel {
    /// Opens `kind` from `line` (replacing any open one).
    pub fn show(&mut self, kind: u8, line: i32) {
        self.open = Some((kind, line));
        self.page = 0;
    }
}

/// One entry: font, centred, text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub font: u32,
    pub centre: bool,
    pub text: String,
}

/// Reads an entry (0x56c9f0): the font number, whitespace, the justification letter, one
/// whitespace character, the text.
pub fn parse_entry(raw: &str) -> Entry {
    let digits = raw.trim_start();
    let end = digits.find(|c: char| !c.is_ascii_digit()).unwrap_or(digits.len());
    let font = digits[..end].parse().unwrap_or(0);
    let rest = digits[end..].trim_start();
    let mut chars = rest.chars();
    let letter = chars.next();
    let mut text = chars.as_str();
    if text.starts_with(char::is_whitespace) {
        text = &text[text.chars().next().map_or(0, char::len_utf8)..];
    }
    Entry { font, centre: matches!(letter, Some('c' | 'C')), text: text.to_string() }
}

/// A block of text on a page: its entry's font and justification, its lines, and how far down
/// the page it starts (font pixels).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub font: u32,
    pub centre: bool,
    pub text: String,
    pub y: u32,
}

/// Lays `entries` over pages `width` x `height` (font pixels): each entry wrapped in its font,
/// stacked under the last, running on to the next page when it does not fit (at most
/// [`MOST_PAGES`]). `font` gives a loaded font by number; an entry without one is left out.
pub fn paginate<'a>(entries: &[Entry], font: impl Fn(u32) -> Option<&'a BitmapFont>, width: u32, height: u32) -> Vec<Vec<Piece>> {
    flow(entries, font, width, |page| (page < MOST_PAGES).then_some(height))
}

/// Lays `entries` down a run of rectangles `width` wide, rectangle `i` `height(i)` tall (font
/// pixels), as [`paginate`] does; stops when `height` has no more rectangles (0x56c800).
pub fn flow<'a>(entries: &[Entry], font: impl Fn(u32) -> Option<&'a BitmapFont>, width: u32, height: impl Fn(usize) -> Option<u32>) -> Vec<Vec<Piece>> {
    let mut pages = vec![Vec::new()];
    let Some(mut room_height) = height(0) else { return Vec::new() };
    let mut y = 0;
    for entry in entries {
        let Some(f) = font(entry.font) else { continue };
        let line_height = f.line_height.max(1);
        let mut lines: Vec<String> = f.layout(&entry.text, Some(width)).into_iter().map(|l| if l.indent > 0 { format!("\t{}", l.text) } else { l.text }).collect();
        while !lines.is_empty() {
            let room = ((room_height.saturating_sub(y)) / line_height) as usize;
            if room == 0 {
                let Some(next) = height(pages.len()) else { return pages };
                pages.push(Vec::new());
                room_height = next;
                y = 0;
                continue;
            }
            let taken: Vec<String> = lines.drain(..room.min(lines.len())).collect();
            let used = taken.len() as u32 * line_height;
            pages.last_mut().expect("a page").push(Piece { font: entry.font, centre: entry.centre, text: taken.join("\n"), y });
            y += used;
        }
    }
    pages
}

/// A label drawn once at a point (0x56c630): font, text, x, y, alignment (0 left, 1 centred
/// on x, 2 ending at x), in the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub font: u32,
    pub text: String,
    pub at: (i32, i32),
    pub align: u8,
}

/// The newspaper's header (table 0x5ca638): font, key, x, y, alignment.
pub const NEWSPAPER_HEADER: [(u32, u32, i32, i32, u8); 6] = [(485, 0, 216, 44, 1), (484, 1, 453, 41, 1), (485, 0, 689, 44, 1), (482, 2, 194, 76, 0), (482, 3, 449, 76, 1), (482, 2, 712, 76, 2)];
/// The headline's rectangle (0x5ca728) and the body's columns (0x5ca738), in the window.
pub const HEADLINE: Rect = Rect { min: Vec2::new(186.0, 97.0), max: Vec2::new(441.0, 187.0) };
pub const COLUMNS: [Rect; 4] = [
    Rect { min: Vec2::new(188.0, 196.0), max: Vec2::new(308.0, 371.0) },
    Rect { min: Vec2::new(328.0, 196.0), max: Vec2::new(448.0, 371.0) },
    Rect { min: Vec2::new(460.0, 103.0), max: Vec2::new(580.0, 373.0) },
    Rect { min: Vec2::new(592.0, 103.0), max: Vec2::new(712.0, 373.0) },
];
/// The filler stories' first key (0x56baa0 counts those from 10 to 999).
pub const FILLER_FROM: u32 = 10;
/// The telegram's labels (table 0x5ca538): font, key, x, y, alignment.
pub const TELEGRAM_LABELS: [(u32, u32, i32, i32, u8); 11] = [
    (481, 1000, 342, 35, 1),
    (481, 1001, 342, 41, 1),
    (481, 1002, 321, 50, 0),
    (481, 1003, 321, 59, 0),
    (481, 1004, 321, 68, 0),
    (479, 1005, 450, 36, 1),
    (479, 1006, 450, 61, 1),
    (481, 1007, 558, 35, 1),
    (481, 1008, 539, 50, 0),
    (481, 1009, 539, 60, 0),
    (481, 1010, 539, 69, 0),
];
/// The telegram's notice (key 1011, font 481, 0x5ca518) and its body (font 480, 0x5ca508).
pub const TELEGRAM_NOTICE: (u32, u32, Rect) = (481, 1011, Rect { min: Vec2::new(335.0, 90.0), max: Vec2::new(569.0, 110.0) });
pub const TELEGRAM_BODY: (u32, Rect) = (480, Rect { min: Vec2::new(327.0, 126.0), max: Vec2::new(577.0, 366.0) });

/// What a written window shows: text flowing through rectangles, and labels.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    /// Each flow: its entries, and its rectangles (a book's: the two pages, again and again).
    pub flows: Vec<(Vec<Entry>, Vec<Rect>, bool)>,
    pub labels: Vec<Label>,
}

/// The layout of `kind` from `line` of `mes` (0x56bcc0 / 0x56c050).
pub fn layout(kind: u8, line: i32, mes: &MesFile) -> Layout {
    let texts = |from: i32, most: u32| entries_from(mes, from, most);
    match i32::from(kind) {
        arcanum_sim::WRITTEN_BOOK => Layout { flows: vec![(texts(line, MOST_ENTRIES).iter().map(|t| parse_entry(t)).collect(), BOOK_PAGES.to_vec(), true)], labels: Vec::new() },
        arcanum_sim::WRITTEN_PLAQUE => Layout { flows: vec![(texts(line, MOST_ENTRIES).iter().map(|t| parse_entry(t)).collect(), vec![PLAQUE_TEXT], false)], labels: Vec::new() },
        arcanum_sim::WRITTEN_NOTE => Layout { flows: vec![(vec![Entry { font: NOTE_FONT, centre: false, text: texts(line, MOST_ENTRIES).join("\n") }], vec![NOTE_TEXT], false)], labels: Vec::new() },
        arcanum_sim::WRITTEN_TELEGRAM => {
            let mut labels: Vec<Label> = TELEGRAM_LABELS.iter().filter_map(|&(font, key, x, y, align)| Some(Label { font, text: mes.get(key)?.to_string(), at: (x, y), align })).collect();
            let notice = mes.get(TELEGRAM_NOTICE.1).map(|t| vec![Entry { font: TELEGRAM_NOTICE.0, centre: false, text: t.to_string() }]).unwrap_or_default();
            let body = vec![Entry { font: TELEGRAM_BODY.0, centre: false, text: texts(line, MOST_ENTRIES).join("\n") }];
            labels.retain(|l| !l.text.is_empty());
            Layout { flows: vec![(notice, vec![TELEGRAM_NOTICE.2], false), (body, vec![TELEGRAM_BODY.1], false)], labels }
        }
        arcanum_sim::WRITTEN_NEWSPAPER => {
            let labels = NEWSPAPER_HEADER.iter().filter_map(|&(font, key, x, y, align)| Some(Label { font, text: mes.get(key)?.to_string(), at: (x, y), align })).collect();
            let headline = texts(line, 1).iter().map(|t| parse_entry(t)).collect();
            // The paper's own stories (at most nine), then filler stories from keys 10 on,
            // starting at line / 10 + 10 and wrapping round (0x56c16f).
            let mut body: Vec<Entry> = texts(line + 1, MOST_ENTRIES - 1).iter().map(|t| parse_entry(t)).collect();
            let fillers = (FILLER_FROM..1000).take_while(|&k| mes.get(k).is_some()).count() as i32;
            if fillers > 0 {
                let start = (line / 10 + 10 - FILLER_FROM as i32).rem_euclid(fillers);
                body.extend((0..fillers).filter_map(|i| mes.get(FILLER_FROM + ((start + i) % fillers) as u32)).map(parse_entry));
            }
            Layout { flows: vec![(headline, vec![HEADLINE], false), (body, COLUMNS.to_vec(), false)], labels }
        }
        _ => Layout::default(),
    }
}

/// The entries of `mes` from `line`: at most `most`, up to the first missing.
pub fn entries_from(mes: &MesFile, line: i32, most: u32) -> Vec<String> {
    let Ok(first) = u32::try_from(line) else { return Vec::new() };
    (first..first + most).map_while(|id| mes.get(id).map(str::to_string)).collect()
}

/// The `.mes` a kind reads.
pub fn mes_file(kind: u8) -> Option<&'static str> {
    match i32::from(kind) {
        arcanum_sim::WRITTEN_BOOK => Some("mes/gamebook.mes"),
        arcanum_sim::WRITTEN_NOTE => Some("mes/gamenote.mes"),
        arcanum_sim::WRITTEN_NEWSPAPER => Some(arcanum_sim::newspaper::NEWSPAPER_MES),
        arcanum_sim::WRITTEN_TELEGRAM => Some("mes/gametelegram.mes"),
        arcanum_sim::WRITTEN_PLAQUE => Some("mes/gameplaque.mes"),
        _ => None,
    }
}

#[derive(Component)]
struct WrittenRoot;

/// The page buttons: true for next.
#[derive(Component, Clone, Copy)]
struct PageButton(bool);

pub struct WrittenWindowPlugin;

impl Plugin for WrittenWindowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WrittenPanel>()
            .add_systems(OnExit(AppState::InGame), |mut panel: ResMut<WrittenPanel>| *panel = WrittenPanel::default())
            .add_systems(Update, (click, show_window).chain().run_if(in_state(AppState::InGame)));
    }
}

/// What the window needs from the game data: the interface art names, the layout and fonts.
#[derive(Default)]
struct Loaded {
    names: Option<MesFile>,
    fonts: std::collections::BTreeMap<u32, Handle<BitmapFontAsset>>,
    layout: Layout,
    /// Each flow's pages.
    pages: Option<Vec<Vec<Vec<Piece>>>>,
    shown: Option<((u8, i32), usize)>,
}

fn interface_file(names: &MesFile, art: u32) -> Option<String> {
    names.get(art).map(|n| format!("{}/{}", interface::DIR, n.trim()))
}

/// A release closes the window, or turns its pages on a page button; Escape closes it.
fn click(buttons: Res<ButtonInput<MouseButton>>, mut keys: ResMut<ButtonInput<KeyCode>>, windows: Query<&Window>, mut panel: ResMut<WrittenPanel>, pages: Query<(&PageButton, &bevy::ui::RelativeCursorPosition)>) {
    if panel.open.is_none() {
        return;
    }
    if keys.clear_just_pressed(KeyCode::Escape) {
        panel.open = None;
        return;
    }
    if !buttons.just_released(MouseButton::Left) || windows.iter().all(|w| w.cursor_position().is_none()) {
        return;
    }
    if let Some((&PageButton(next), _)) = pages.iter().find(|(_, r)| r.cursor_over()) {
        panel.page = if next { panel.page + 2 } else { panel.page.saturating_sub(2) };
    } else {
        panel.open = None;
    }
}

/// Draws the window as it should be, once its fonts are in.
#[allow(clippy::too_many_arguments)]
fn show_window(
    mut commands: Commands,
    panel: Res<WrittenPanel>,
    roots: Query<Entity, With<WrittenRoot>>,
    assets: Res<AssetServer>,
    fonts: Res<Assets<BitmapFontAsset>>,
    vfs: Option<Res<VfsResource>>,
    mut loaded: Local<Loaded>,
) {
    let want = panel.open.map(|o| (o, panel.page));
    if loaded.shown == want && (want.is_none() || !roots.is_empty()) {
        return;
    }
    let Some(((kind, line), page)) = want else {
        for root in &roots {
            commands.entity(root).despawn();
        }
        loaded.shown = None;
        return;
    };
    // A new text: read it and ask for its fonts.
    if loaded.shown.map(|s| s.0) != Some((kind, line)) {
        let guard = vfs.as_ref().and_then(|v| v.0.read().ok());
        let read = |path: &str| guard.as_ref().and_then(|v| v.read(path).ok()).and_then(|d| MesFile::parse(&d).ok());
        if loaded.names.is_none() {
            loaded.names = read(interface::INDEX);
        }
        loaded.layout = mes_file(kind).and_then(read).map(|m| layout(kind, line, &m)).unwrap_or_default();
        let wanted: std::collections::BTreeSet<u32> = loaded.layout.flows.iter().flat_map(|f| f.0.iter().map(|e| e.font)).chain(loaded.layout.labels.iter().map(|l| l.font)).collect();
        let names = loaded.names.clone().unwrap_or_default();
        loaded.fonts = wanted.into_iter().filter_map(|f| Some((f, assets.load(interface_file(&names, f)?)))).collect();
        loaded.pages = None;
        loaded.shown = None;
    }
    if loaded.pages.is_none() {
        let pending = loaded.fonts.values().any(|h| !fonts.contains(h) && !assets.load_state(h).is_failed());
        if pending {
            return;
        }
        let by_number: std::collections::BTreeMap<u32, &BitmapFont> = loaded.fonts.iter().filter_map(|(&n, h)| Some((n, &fonts.get(h)?.0))).collect();
        let pages = loaded
            .layout
            .flows
            .iter()
            .map(|(entries, rects, repeats)| {
                let width = rects.first().map_or(0.0, |r| r.width()) as u32;
                let height = |i: usize| if *repeats { (i < MOST_PAGES).then(|| rects[i % rects.len()].height() as u32) } else { rects.get(i).map(|r| r.height() as u32) };
                flow(entries, |n| by_number.get(&n).copied(), width, height)
            })
            .collect();
        loaded.pages = Some(pages);
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    loaded.shown = want;
    let pages = loaded.pages.clone().unwrap_or_default();
    let names = loaded.names.clone().unwrap_or_default();
    let art = |n: u32| interface_file(&names, n).map(|p| assets.load::<Image>(p));
    let window = |r: Rect| Rect { min: r.min + WINDOW.min, max: r.max + WINDOW.min };
    let size = |n: u32| art_size(vfs.as_deref(), &names, n);
    let background = if i32::from(kind) == arcanum_sim::WRITTEN_IMAGE { u32::try_from(line).unwrap_or(0) } else { BACKGROUNDS.get(usize::from(kind)).copied().unwrap_or(0) };
    let colour = if i32::from(kind) == arcanum_sim::WRITTEN_PLAQUE { Color::WHITE } else { Color::BLACK };
    let book = i32::from(kind) == arcanum_sim::WRITTEN_BOOK;
    let measure = |font: u32, text: &str| loaded.fonts.get(&font).and_then(|h| fonts.get(h)).map_or(0, |f| f.0.measure(text)) as f32;
    commands
        .spawn((
            WrittenRoot,
            DespawnOnExit(AppState::InGame),
            GlobalZIndex(5),
            Node { position_type: PositionType::Absolute, top: Val::Px(0.0), left: Val::Percent(50.0), margin: UiRect::left(crate::menu_window::px(-400.0)), width: crate::menu_window::px(800.0), height: crate::menu_window::px(600.0), ..default() },
        ))
        .with_children(|screen| {
            for (n, corner) in [(BACKDROP_ART, Vec2::ZERO), (background, BACKGROUND_AT)] {
                if let (Some(image), Some(s)) = (art(n), size(n)) {
                    screen.spawn((bevy::picking::Pickable::IGNORE, ImageNode::new(image), at(window(rect(corner, s)))));
                }
            }
            let mut text = |font: u32, body: String, place: Rect, align: Align| {
                let Some(handle) = loaded.fonts.get(&font) else { return };
                let t = BitmapText { shadow: false, ..BitmapText::new(body, handle.clone(), ORIGINAL_SCALE) }.wrapped(place.width() * ORIGINAL_SCALE, align);
                screen.spawn((bevy::picking::Pickable::IGNORE, TextColor(colour), t, at(window(place))));
            };
            for ((_, rects, repeats), flow_pages) in loaded.layout.flows.iter().zip(&pages) {
                let shown: Vec<(Rect, Option<&Vec<Piece>>)> = if *repeats { rects.iter().enumerate().map(|(i, r)| (*r, flow_pages.get(page + i))).collect() } else { rects.iter().enumerate().map(|(i, r)| (*r, flow_pages.get(i))).collect() };
                for (place, pieces) in shown {
                    for piece in pieces.into_iter().flatten() {
                        let corner = place.min + Vec2::new(0.0, piece.y as f32);
                        text(piece.font, piece.text.clone(), rect(corner, Vec2::new(place.width(), place.max.y - corner.y)), if piece.centre { Align::Center } else { Align::Left });
                    }
                }
            }
            for label in &loaded.layout.labels {
                let w = measure(label.font, &label.text);
                let x = match label.align {
                    1 => label.at.0 as f32 - w / 2.0,
                    2 => label.at.0 as f32 - w,
                    _ => label.at.0 as f32,
                };
                text(label.font, label.text.clone(), rect(Vec2::new(x, label.at.1 as f32), Vec2::new(w + 1.0, 30.0)), Align::Left);
            }
            if book {
                let next_shown = pages.first().is_some_and(|p| page + 2 < p.len());
                for ((n, corner), next, shown) in [(PREVIOUS, false, page > 0), (NEXT, true, next_shown)] {
                    if let (true, Some(image), Some(s)) = (shown, art(n), size(n)) {
                        screen.spawn((PageButton(next), bevy::ui::RelativeCursorPosition::default(), ImageNode::new(image), at(window(rect(corner, s)))));
                    }
                }
            }
        });
}

/// An interface art's size (its first frame), in original pixels.
fn art_size(vfs: Option<&VfsResource>, names: &MesFile, n: u32) -> Option<Vec2> {
    let path = interface_file(names, n)?;
    let guard = vfs?.0.read().ok()?;
    let art = arcanum_formats::art::ArtFile::parse(&guard.read(&path).ok()?).ok()?;
    let frame = art.frames.first()?.first()?;
    Some(Vec2::new(frame.width as f32, frame.height as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_start_with_a_font_and_a_justification() {
        assert_eq!(parse_entry("28c\nOffer to him\nWho"), Entry { font: 28, centre: true, text: "Offer to him\nWho".into() });
        assert_eq!(parse_entry("497l \tMay 11, 1823"), Entry { font: 497, centre: false, text: "\tMay 11, 1823".into() });
        assert_eq!(parse_entry("37r Right"), Entry { font: 37, centre: false, text: "Right".into() }, "only c centres");
        assert_eq!(parse_entry("744c 5"), Entry { font: 744, centre: true, text: "5".into() });
    }

    #[test]
    fn entries_run_from_the_line_to_the_first_gap() {
        let mes = MesFile::parse(b"{1000}{37l a}{1001}{37l b}{1003}{37l d}").unwrap();
        assert_eq!(entries_from(&mes, 1000, MOST_ENTRIES), ["37l a", "37l b"]);
        assert_eq!(entries_from(&mes, 1003, 1), ["37l d"]);
        assert!(entries_from(&mes, -1, 5).is_empty());
    }

    #[test]
    fn pages_fill_and_run_on() {
        let font = BitmapFont::parse(&arcanum_formats::test_util::font_art(10)).unwrap();
        let words = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
        let entries = vec![Entry { font: 1, centre: false, text: words.into() }, Entry { font: 1, centre: true, text: "omega".into() }, Entry { font: 2, centre: false, text: "no font".into() }];
        let width = font.measure("epsilon zeta");
        let pages = paginate(&entries, |n| (n == 1).then_some(&font), width, 30);
        assert!(pages.len() > 1, "{pages:?}");
        for page in &pages {
            for piece in page {
                assert!(piece.y + piece.text.lines().count() as u32 * 10 <= 30, "{piece:?} fits its page");
            }
        }
        let all: Vec<&str> = pages.iter().flatten().flat_map(|p| p.text.split_whitespace()).collect();
        assert_eq!(all.join(" "), format!("{words} omega"), "every word once, in order; the entry without a font left out");
        let last = pages.last().unwrap().last().unwrap();
        assert!(last.centre && last.text == "omega");
    }

    #[test]
    fn a_newspaper_has_its_header_headline_stories_then_fillers() {
        let mes = MesFile::parse(b"{0}{EXTRA!}{1}{[ THE TARANTIAN ]}{2}{Price}{3}{Motto}{10}{492c _ WEATHER _}{11}{483l Sunny.}{12}{492c _ TAX _}{5000}{490c HEADLINE}{5001}{492c _ SUB _}{5002}{483l Story.}").unwrap();
        let l = layout(arcanum_sim::WRITTEN_NEWSPAPER as u8, 5000, &mes);
        assert_eq!(l.labels.len(), 6);
        assert_eq!((l.labels[1].font, l.labels[1].text.as_str(), l.labels[1].at, l.labels[1].align), (484, "[ THE TARANTIAN ]", (453, 41), 1));
        assert_eq!(l.flows[0].0, [Entry { font: 490, centre: true, text: "HEADLINE".into() }]);
        let body: Vec<&str> = l.flows[1].0.iter().map(|e| e.text.as_str()).collect();
        // Fillers from 5000 / 10 + 10 = 510, round the three (10-12): 510 - 10 = 500, 500 % 3 = 2.
        assert_eq!(body, ["_ SUB _", "Story.", "_ TAX _", "_ WEATHER _", "Sunny."]);
        assert_eq!(l.flows[1].1.len(), 4, "four columns");
    }

    #[test]
    fn a_telegram_has_its_labels_notice_and_body_in_its_fonts() {
        let mes = MesFile::parse(b"{1000}{CLASS OF}{1011}{NO RESPONSIBILITY}{1020}{Mr. Ristezze}{1021}{Enclosed.}").unwrap();
        let l = layout(arcanum_sim::WRITTEN_TELEGRAM as u8, 1020, &mes);
        assert_eq!(l.labels, [Label { font: 481, text: "CLASS OF".into(), at: (342, 35), align: 1 }]);
        assert_eq!(l.flows[0].0[0].text, "NO RESPONSIBILITY");
        assert_eq!(l.flows[1].0, [Entry { font: 480, centre: false, text: "Mr. Ristezze\nEnclosed.".into() }]);
    }
}
