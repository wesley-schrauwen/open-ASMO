//! The in-game interface: the original's top and bottom bars, with the local PC's health and
//! fatigue vials.
//!
//! From Arcanum.exe (interface art ids from `art/interface/interface.mes`):
//! - Two windows (table 0x5c6390, created at 0x54a406): the top bar (0, 0, 800, 41, art 185)
//!   and the bottom bar (0, 441, 800, 159, art 184). The original draws the world only in the
//!   800x400 band between them (0x557810), and the all-in-one `intrface.art` (art 3) is never
//!   used.
//! - Top bar buttons, 34x34 at y 2: character (169) at x 4, log (187) 41, town map (193) 78,
//!   inventory (186) 115; five maintained-spell slots (188-192) at x 281 + 50 n, y 3; the
//!   clock ([`crate::hud_clock`]).
//! - Bottom bar buttons, 39x39 (table 0x5c6470..0x5c64b0): combat (470) at (86, 457), skills
//!   (472) at (693, 456), spells (473) at (649, 494), schematics (471) at (693, 539); ten hotkey
//!   slots, 32x32 at y 445 (0x5cb494, 0x5cb4bc).
//! - The vials ([`vial`], 0x54af10): health at (14, 472, 28, 88), fatigue at (754, 473, 28, 88),
//!   with the current value under each (`%03d` in Cloister18Font, 0x54ad00). The text colour
//!   was not found; it is the font's own white here.
//!
//! Owner's layout (2026-11-21): the world fills the window; the two bars are centred on its
//! top and bottom edges, in original pixels scaled like the menus, so a wide window shows world
//! beside them. Clicks on the bars, and on the character screen while it is open, do not reach the world
//! ([`covers`]).
//!
//! Not built: what the buttons do (they only show their hover and pressed frames; the hotkey
//! slots are [`crate::hotkey_bar`]),
//! the message box, the ammo icon and the magick
//! weapon button, and the poisoned (yellow) health vial (nothing poisons yet).

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{ActivateOnPress, Button};

use crate::message_box::BoxHelp;
use crate::actors::Party;
use arcanum_sim::Pool;

use crate::art_loader::ArtFrames;
use crate::bitmap_text::{BitmapFontAsset, BitmapText};
use crate::fonts::ORIGINAL_SCALE;
use crate::main_menu::AppState;
use crate::menu_window::{at, px, rect};
use crate::resource_pack::paths::{fonts, interface};
use crate::sim_bridge::LocalPc;
use crate::widgets::FrameButton;

/// The bars' size and where the bottom one starts, in original pixels.
pub const BAR_WIDTH: f32 = 800.0;
pub const TOP_HEIGHT: f32 = 41.0;
pub const BOTTOM_HEIGHT: f32 = 159.0;
pub const BOTTOM_TOP: f32 = 441.0;

/// Health and fatigue vial rects (screen coordinates), and the height the fill spans.
pub const HEALTH_VIAL: Rect = Rect { min: Vec2::new(14.0, 472.0), max: Vec2::new(42.0, 560.0) };
pub const FATIGUE_VIAL: Rect = Rect { min: Vec2::new(754.0, 473.0), max: Vec2::new(782.0, 561.0) };
pub const VIAL_HEIGHT: i32 = 88;
/// Where the vials' numbers go (0x5c63f8), one pixel taller than the table says, as drawn.
pub const HEALTH_NUMBER: Rect = Rect { min: Vec2::new(15.0, 578.0), max: Vec2::new(44.0, 591.0) };
pub const FATIGUE_NUMBER: Rect = Rect { min: Vec2::new(755.0, 578.0), max: Vec2::new(782.0, 591.0) };

/// Top bar buttons: file, x, hover help (`mes/intgame.mes`, 0x54b5d0).
const TOP_BUTTONS: [(&str, f32, u32); 4] = [("Char_But.ART", 4.0, 1005), ("Log_But.ART", 41.0, 1004), ("TMap_But.ART", 78.0, 1007), ("Invn_But.ART", 115.0, 1006)];
/// Bottom bar buttons: file, top-left corner, hover help.
const BOTTOM_BUTTONS: [(&str, Vec2, u32); 4] = [
    ("Combat_Button.ART", Vec2::new(86.0, 457.0), 1002),
    ("Skills_Button.ART", Vec2::new(693.0, 456.0), 1001),
    ("Spells_Button.ART", Vec2::new(649.0, 494.0), 1000),
    ("Schematics_Button.ART", Vec2::new(693.0, 539.0), 1003),
];
/// Hotkey slots 1-9 and 0, left to right ([`crate::hotkey_bar`]).
pub const HOTKEY_X: [f32; 10] = [198.0, 237.0, 276.0, 315.0, 354.0, 418.0, 456.0, 495.0, 534.0, 573.0];
pub const HOTKEY_Y: f32 = 445.0;

/// What a bar button does (the rest do nothing yet).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudAction {
    /// Opens or closes the character screen ([`crate::character_editor::CharacterPanel`]).
    Character,
    /// Opens or closes the inventory screen ([`crate::inventory_screen::InventoryPanel`]).
    Inventory,
}

/// Which pool a vial shows.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vial {
    Health,
    Fatigue,
}

/// The coloured part of a vial (the empty glass is its sibling, [`VialGlass`]).
#[derive(Component, Debug, Clone, Copy)]
pub struct VialFill(pub Vial);

/// The empty glass above a vial's fill.
#[derive(Component, Debug, Clone, Copy)]
pub struct VialGlass(pub Vial);

/// The number under a vial.
#[derive(Component, Debug, Clone, Copy)]
pub struct VialNumber(pub Vial);

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::message_box::MessageBoxPlugin>() {
            app.add_plugins(crate::message_box::MessageBoxPlugin);
        }
        app.add_systems(OnEnter(AppState::InGame), spawn_hud).add_systems(Update, (show_vials, crate::hud_clock::show_clock).run_if(in_state(AppState::InGame)));
    }
}

fn art(name: &str) -> String {
    format!("{}/{name}", interface::DIR)
}

/// A bar, `height` original pixels tall, centred on the window's top (`top`) or bottom edge.
/// Black shows through the holes in its art, as in the original's windows.
fn bar(top: bool, height: f32) -> impl Bundle {
    let (top, bottom) = if top { (Val::Px(0.0), Val::Auto) } else { (Val::Auto, Val::Px(0.0)) };
    (
        BackgroundColor(Color::BLACK),
        Node { position_type: PositionType::Absolute, top, bottom, left: Val::Percent(50.0), margin: UiRect::left(px(-BAR_WIDTH / 2.0)), width: px(BAR_WIDTH), height: px(height), ..default() },
    )
}

/// An art button at `rect` (bar coordinates): normal, pressed and hover frames. It does nothing
/// yet.
fn hud_button(frames: Handle<ArtFrames>, rect: Rect) -> impl Bundle {
    (Button, ActivateOnPress, Hovered::default(), FrameButton::arrow(frames), ImageNode::default(), at(rect))
}

fn spawn_hud(mut commands: Commands, assets: Res<AssetServer>) {
    commands
        .spawn((DespawnOnExit(AppState::InGame), bar(true, TOP_HEIGHT), ImageNode::new(assets.load(interface::HUD_TOP))))
        .with_children(|top| {
            for (i, (file, x, help)) in TOP_BUTTONS.into_iter().enumerate() {
                let mut button = top.spawn((BoxHelp::Intgame(help), hud_button(assets.load(art(file)), rect(Vec2::new(x, 2.0), Vec2::splat(34.0)))));
                match i {
                    0 => {
                        button.insert(HudAction::Character);
                    }
                    3 => {
                        button.insert(HudAction::Inventory);
                    }
                    _ => {}
                }
            }
            for n in 0..5 {
                top.spawn((ImageNode::new(assets.load(art(&format!("SplSlt{}.ART", n + 1)))), at(rect(Vec2::new(281.0 + 50.0 * n as f32, 3.0), Vec2::splat(32.0)))));
            }
            crate::hud_clock::spawn_clock(top, &assets);
        });
    let local = |r: Rect| Rect { min: r.min - Vec2::new(0.0, BOTTOM_TOP), max: r.max - Vec2::new(0.0, BOTTOM_TOP) };
    commands
        .spawn((DespawnOnExit(AppState::InGame), bar(false, BOTTOM_HEIGHT), ImageNode::new(assets.load(interface::HUD_BOTTOM))))
        .with_children(|bottom| {
            for (file, corner, help) in BOTTOM_BUTTONS {
                bottom.spawn((BoxHelp::Intgame(help), hud_button(assets.load(art(file)), local(rect(corner, Vec2::splat(39.0))))));
            }
            crate::message_box::spawn_box(bottom, local);
            for (slot, x) in HOTKEY_X.into_iter().enumerate() {
                let file = format!("HKSlt{}.ART", (slot + 1) % 10);
                bottom.spawn((crate::hotkey_bar::HotkeySlot(slot), ImageNode::new(assets.load(art(&file))), at(local(rect(Vec2::new(x, HOTKEY_Y), Vec2::splat(32.0))))));
            }
            let digits: Handle<BitmapFontAsset> = assets.load(fonts::HUD_NUMBERS);
            for (vial, area, fill, number) in [(Vial::Health, HEALTH_VIAL, interface::VIAL_RED, HEALTH_NUMBER), (Vial::Fatigue, FATIGUE_VIAL, interface::VIAL_BLUE, FATIGUE_NUMBER)] {
                let area = local(area);
                // The glass, then the liquid over it (its surface overlaps the glass).
                bottom.spawn((VialGlass(vial), ImageNode::new(assets.load(interface::VIAL_EMPTY)), at(area)));
                bottom.spawn((VialFill(vial), ImageNode::new(assets.load(fill)), at(area)));
                // The number, centred on a black box.
                bottom.spawn((BackgroundColor(Color::BLACK), Node { justify_content: JustifyContent::Center, align_items: AlignItems::Center, overflow: Overflow::clip(), ..at(local(number)) })).with_children(|b| {
                    b.spawn((VialNumber(vial), BitmapText { shadow: false, ..BitmapText::new("", digits.clone(), ORIGINAL_SCALE) }));
                });
            }
        });
}

/// The rows a vial shows for `current` of `max` (0x54af10): the fill is `100 * current / max`
/// percent (50 when the maximum is 0) of [`VIAL_HEIGHT`], rounded down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VialRows {
    /// Rows of empty glass at the top (`clrvial`, from its top-left).
    pub empty: i32,
    /// Where the coloured art is drawn from the vial's top, how many rows, and from which row of
    /// the art. Its top 8 rows are the liquid's surface, drawn over the glass above the fill.
    pub liquid_top: i32,
    pub liquid_rows: i32,
    pub liquid_from: i32,
}

pub fn vial(current: i32, max: i32) -> VialRows {
    let percent = if max == 0 { 50 } else { (100 * current / max).clamp(0, 100) };
    let fill = VIAL_HEIGHT * percent / 100;
    let empty = VIAL_HEIGHT - fill;
    if fill == 0 {
        return VialRows { empty, liquid_top: 0, liquid_rows: 0, liquid_from: 0 };
    }
    VialRows { empty, liquid_top: (empty - 8).max(0), liquid_rows: (fill + 8).min(VIAL_HEIGHT), liquid_from: 8 - empty.min(8) }
}

/// The number under a vial: the current value, three digits (`%03d`, 0x54ad00).
pub fn vial_number(current: i32) -> String {
    format!("{current:03}")
}

fn vial_area(vial: Vial) -> Rect {
    match vial {
        Vial::Health => HEALTH_VIAL,
        Vial::Fatigue => FATIGUE_VIAL,
    }
}

/// Draws each vial and its number from the local PC's pools (empty until the host sends them).
#[allow(clippy::type_complexity)]
fn show_vials(
    pc: Res<LocalPc>,
    party: Res<Party>,
    mut fills: Query<(&VialFill, &mut Node, &mut ImageNode, &mut Visibility), (Without<VialGlass>, Without<VialNumber>)>,
    mut glasses: Query<(&VialGlass, &mut Node, &mut ImageNode, &mut Visibility), (Without<VialFill>, Without<VialNumber>)>,
    mut numbers: Query<(&VialNumber, &mut BitmapText)>,
) {
    let status = pc.0.and_then(|(id, _)| party.status.get(&id));
    let pool = |vial: Vial| match (status, vial) {
        (Some(s), Vial::Health) => s.hit_points,
        (Some(s), Vial::Fatigue) => s.fatigue,
        (None, _) => Pool { current: 0, max: 1 },
    };
    for (&VialFill(which), mut node, mut image, mut visibility) in &mut fills {
        let rows = vial(pool(which).current, pool(which).max);
        let top = px(vial_area(which).min.y - BOTTOM_TOP + rows.liquid_top as f32);
        let height = px(rows.liquid_rows as f32);
        let source = Some(Rect::new(0.0, rows.liquid_from as f32, 28.0, (rows.liquid_from + rows.liquid_rows) as f32));
        if node.top != top || node.height != height || image.rect != source {
            (node.top, node.height, image.rect) = (top, height, source);
        }
        visibility.set_if_neq(if rows.liquid_rows > 0 { Visibility::Inherited } else { Visibility::Hidden });
    }
    for (&VialGlass(which), mut node, mut image, mut visibility) in &mut glasses {
        let rows = vial(pool(which).current, pool(which).max);
        let height = px(rows.empty as f32);
        let source = Some(Rect::new(0.0, 0.0, 28.0, rows.empty as f32));
        if node.height != height || image.rect != source {
            (node.height, image.rect) = (height, source);
        }
        visibility.set_if_neq(if rows.empty > 0 { Visibility::Inherited } else { Visibility::Hidden });
    }
    for (&VialNumber(vial), mut text) in &mut numbers {
        let wanted = if status.is_some() { vial_number(pool(vial).current) } else { String::new() };
        if text.text != wanted {
            text.text = wanted;
        }
    }
}

/// Whether the bars cover `cursor` (logical window pixels) in a `window_size` window: there a
/// click is the interface's, not the world's.
/// With `panel_open`, the character screen between them covers the 800x400 band too.
pub fn covers(window_size: Vec2, cursor: Vec2, panel_open: bool) -> bool {
    let scale = window_size.y / 600.0;
    let half = BAR_WIDTH / 2.0 * scale;
    (cursor.x - window_size.x / 2.0).abs() <= half && (panel_open || cursor.y < TOP_HEIGHT * scale || cursor.y >= window_size.y - BOTTOM_HEIGHT * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vials_fill_from_the_bottom_as_the_original() {
        let rows = |c, m| {
            let r = vial(c, m);
            (r.empty, r.liquid_top, r.liquid_rows, r.liquid_from)
        };
        assert_eq!(rows(30, 30), (0, 0, 88, 8), "full: the art below its surface");
        assert_eq!(rows(15, 30), (44, 36, 52, 0), "half: the surface 8 rows over the glass");
        assert_eq!(rows(1, 30), (86, 78, 10, 0), "3 percent of 88, rounded down");
        assert_eq!(rows(29, 30), (4, 0, 88, 4), "96 percent: the surface is cut");
        assert_eq!(rows(0, 30), (88, 0, 0, 0));
        assert_eq!(rows(5, 0), (44, 36, 52, 0), "no maximum: half");
        assert_eq!(rows(-4, 30).2, 0);
        assert_eq!((vial_number(7), vial_number(123)), ("007".to_string(), "123".to_string()));
    }

    #[test]
    fn the_bars_take_clicks_only_where_they_are() {
        let window = Vec2::new(1920.0, 1080.0);
        // 1.8 window pixels per original pixel: the bars span x 240..1680.
        let covers = |w, c| super::covers(w, c, false);
        assert!(covers(window, Vec2::new(960.0, 10.0)));
        assert!(covers(window, Vec2::new(250.0, 1070.0)));
        assert!(!covers(window, Vec2::new(100.0, 1070.0)), "beside the bottom bar: world");
        assert!(!covers(window, Vec2::new(960.0, 540.0)));
        assert!(covers(window, Vec2::new(960.0, 1080.0 - 159.0 * 1.8 + 1.0)));
        assert!(!covers(window, Vec2::new(960.0, 1080.0 - 159.0 * 1.8 - 1.0)));
        // The character screen takes the band between them, not beside it.
        assert!(super::covers(window, Vec2::new(960.0, 540.0), true));
        assert!(!super::covers(window, Vec2::new(100.0, 540.0), true));
    }
}
