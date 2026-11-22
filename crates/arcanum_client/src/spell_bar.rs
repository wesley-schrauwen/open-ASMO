//! The bottom bar's spells view (Arcanum.exe 0x5506c0 / 0x551210 / 0x5501c0 / 0x550bd0 /
//! 0x550c60 / 0x54b5d0).
//!
//! - **Opening:** the spells button swaps the message box for the view, and again back
//!   (0x5506c0(1) toggles view 1). The view also closes when the cursor leaves the bar upwards
//!   (0x54b5d0), so the world can be clicked.
//! - **Colleges:** a button (`s_<college>.art`, art 11, 44, ...) at y 497, x 201 to 575 about
//!   25 apart (0x5c6618), shown only for colleges the PC knows a spell of.
//! - **Spells:** the chosen college's known spells (`s_<college>n.art`), at y 528, x 284, 334,
//!   384, 435, 485 (0x5c6718), stopping at the first not known (0x550c60).
//! - **Casting:** a click on a spell (on release, 0x54b5d0 state 1) arms it ([`crate::skill_use`]);
//!   the next click on the world casts it (0x57bc70 → 0x57c110). The view stays open.
//!
//! Builder's call: the first college the PC knows is chosen when none is (the original keeps
//! the last one chosen, 0x64c530, its start not traced).
//!
//! Not built: the hover names (the original shows them in the view, 0x550860 / 0x5507e0); the
//! skills view (view 2) and the others; dragging a spell onto the hotkey bar.

use bevy::prelude::*;

use arcanum_rules::{COLLEGES, SPELLS_PER_COLLEGE};

use crate::hud::{BOTTOM_TOP, HudAction};
use crate::inventory_screen::screen_point;
use crate::main_menu::AppState;
use crate::menu_window::{at, rect};
use crate::message_box::MessageBoxRoot;
use crate::resource_pack::paths::interface;

/// The college buttons' x (0x5c6618), at [`COLLEGE_Y`].
pub const COLLEGE_X: [f32; COLLEGES] = [201.0, 226.0, 251.0, 276.0, 301.0, 326.0, 351.0, 376.0, 400.0, 425.0, 450.0, 475.0, 500.0, 525.0, 550.0, 575.0];
pub const COLLEGE_Y: f32 = 497.0;
/// The college buttons' size.
pub const COLLEGE_SIZE: f32 = 24.0;
/// The spell icons' x (0x5c6718), at [`SPELL_Y`].
pub const SPELL_X: [f32; SPELLS_PER_COLLEGE] = [284.0, 334.0, 384.0, 435.0, 485.0];
pub const SPELL_Y: f32 = 528.0;
pub const SPELL_SIZE: f32 = 32.0;

/// Which view the bottom bar's centre shows.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BarView {
    /// The spells view instead of the message box.
    pub spells: bool,
    /// The college whose spells show.
    pub college: Option<usize>,
}

/// Something drawn in the spells view.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpellViewPart {
    College(usize),
    Spell(usize),
}

pub struct SpellBarPlugin;

impl Plugin for SpellBarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BarView>()
            .add_systems(OnExit(AppState::InGame), |mut view: ResMut<BarView>| *view = BarView::default())
            .add_systems(Update, (close_above_the_bar, click_view, show_view).chain().run_if(in_state(AppState::InGame)))
            .add_observer(on_spells_button);
    }
}

/// The spells known in `college` by a PC knowing `known` there, in order.
pub fn shown_spells(college: usize, known: i32) -> impl Iterator<Item = usize> {
    (0..known.clamp(0, SPELLS_PER_COLLEGE as i32) as usize).map(move |k| college * SPELLS_PER_COLLEGE + k)
}

/// What of the view is at `point` (800x600 screen pixels) for a PC knowing `spells` per college
/// with `college` chosen.
pub fn part_at(point: Vec2, spells: &[i32; COLLEGES], college: Option<usize>) -> Option<SpellViewPart> {
    let inside = |x: f32, y: f32, size: f32| (x..x + size).contains(&point.x) && (y..y + size).contains(&point.y);
    if let Some(c) = (0..COLLEGES).find(|&c| spells[c] > 0 && inside(COLLEGE_X[c], COLLEGE_Y, COLLEGE_SIZE)) {
        return Some(SpellViewPart::College(c));
    }
    let college = college?;
    shown_spells(college, spells[college]).zip(SPELL_X).find(|&(_, x)| inside(x, SPELL_Y, SPELL_SIZE)).map(|(s, _)| SpellViewPart::Spell(s))
}

fn on_spells_button(activate: On<bevy::ui_widgets::Activate>, buttons: Query<&HudAction>, mut view: ResMut<BarView>) {
    if buttons.get(activate.entity) == Ok(&HudAction::Spells) {
        view.spells = !view.spells;
    }
}

fn cursor(windows: &Query<&Window>) -> Option<Vec2> {
    windows.iter().find_map(|w| w.cursor_position().map(|c| screen_point(w.size(), c)))
}

/// The view closes when the cursor goes up past the bar's top (0x54b5d0).
fn close_above_the_bar(windows: Query<&Window>, mut view: ResMut<BarView>) {
    if view.spells && cursor(&windows).is_some_and(|p| p.y < BOTTOM_TOP) {
        view.spells = false;
    }
}

/// Press on a college shows its spells; release on a spell arms it.
fn click_view(buttons: Res<ButtonInput<MouseButton>>, windows: Query<&Window>, mut view: ResMut<BarView>, sheet: Res<crate::pc_sheet::SheetHost>, armed: Option<ResMut<crate::skill_use::SkillTargeting>>) {
    if !view.spells || !(buttons.just_pressed(MouseButton::Left) || buttons.just_released(MouseButton::Left)) {
        return;
    }
    let (Some(point), Some(sheet)) = (cursor(&windows), sheet.sheet()) else { return };
    match part_at(point, &sheet.spells, view.college) {
        Some(SpellViewPart::College(c)) if buttons.just_pressed(MouseButton::Left) => view.college = Some(c),
        Some(SpellViewPart::Spell(s)) if buttons.just_released(MouseButton::Left) => {
            if let Some(mut armed) = armed {
                armed.0 = Some(crate::skill_use::Armed::Spell(s));
            }
        }
        _ => {}
    }
}

/// Draws the view over the message box's place, or the message box.
#[allow(clippy::too_many_arguments)]
fn show_view(
    mut commands: Commands,
    mut view: ResMut<BarView>,
    sheet: Res<crate::pc_sheet::SheetHost>,
    assets: Res<AssetServer>,
    mut boxes: Query<(&mut Visibility, &ChildOf), With<MessageBoxRoot>>,
    parts: Query<Entity, With<SpellViewPart>>,
    added: Query<(), Added<MessageBoxRoot>>,
) {
    if !view.is_changed() && !sheet.is_changed() && added.is_empty() {
        return;
    }
    let spells = sheet.sheet().map(|s| s.spells).unwrap_or_default();
    if view.spells && view.college.is_none_or(|c| spells[c] <= 0) {
        let first = (0..COLLEGES).find(|&c| spells[c] > 0);
        if view.college != first {
            view.college = first;
        }
    }
    for e in &parts {
        commands.entity(e).despawn();
    }
    let Ok((mut visibility, parent)) = boxes.single_mut() else { return };
    *visibility = if view.spells { Visibility::Hidden } else { Visibility::Inherited };
    if !view.spells {
        return;
    }
    let local = |x: f32, y: f32, size: f32| at(rect(Vec2::new(x, y - BOTTOM_TOP), Vec2::splat(size)));
    let mut spawned = Vec::new();
    for (c, path) in crate::editor_tabs::college_buttons().into_iter().enumerate().filter(|(c, _)| spells[*c] > 0) {
        spawned.push(commands.spawn((SpellViewPart::College(c), bevy::picking::Pickable::IGNORE, ImageNode::new(assets.load(path)), local(COLLEGE_X[c], COLLEGE_Y, COLLEGE_SIZE))).id());
    }
    if let Some(college) = view.college {
        let stem = crate::editor_tabs::COLLEGE_STEMS[college];
        for (spell, x) in shown_spells(college, spells[college]).zip(SPELL_X) {
            let path = format!("{}/s_{stem}{}.art", interface::DIR, spell % SPELLS_PER_COLLEGE + 1);
            spawned.push(commands.spawn((SpellViewPart::Spell(spell), bevy::picking::Pickable::IGNORE, ImageNode::new(assets.load(path)), local(x, SPELL_Y, SPELL_SIZE))).id());
        }
    }
    commands.entity(parent.parent()).add_children(&spawned);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_view_shows_known_colleges_and_their_known_spells() {
        let mut spells = [0; COLLEGES];
        spells[0] = 2;
        spells[15] = 5;
        assert_eq!(part_at(Vec2::new(205.0, 500.0), &spells, None), Some(SpellViewPart::College(0)));
        assert_eq!(part_at(Vec2::new(230.0, 500.0), &spells, None), None, "no spell known in divination");
        assert_eq!(part_at(Vec2::new(580.0, 510.0), &spells, None), Some(SpellViewPart::College(15)));
        assert_eq!(part_at(Vec2::new(340.0, 540.0), &spells, Some(0)), Some(SpellViewPart::Spell(1)), "the Unlocking Cantrip");
        assert_eq!(part_at(Vec2::new(390.0, 540.0), &spells, Some(0)), None, "the third is not known");
        assert_eq!(part_at(Vec2::new(490.0, 540.0), &spells, Some(15)), Some(SpellViewPart::Spell(79)));
        assert_eq!(part_at(Vec2::new(340.0, 540.0), &spells, None), None);
    }
}
