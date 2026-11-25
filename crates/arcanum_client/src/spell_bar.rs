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
//! - **Dragging:** a spell dragged onto a hotkey slot goes there ([`crate::hotkey_bar`]); its
//!   icon follows the cursor.
//!
//! Builder's call: the first college the PC knows is chosen when none is (the original keeps
//! the last one chosen, 0x64c530, its start not traced).
//!
//! **The skills view** (view 2: the skills button, 0x5506c0(2); buttons 0x5501c0 / 0x551210,
//! table 0x5c6c68): four buttons, 32x32 at y 514, x 288, 351, 414, 477: Prowling
//! (`SKL_Conceal.ART`, art 279), Pick Pocket (280), Disarm Traps (282), Repair (278); skills
//! 6, 5, 15, 12 (table 0x5cb240). All four always show. A click (0x57efa0 → 0x579fa0 →
//! 0x57a0a0) arms Disarm Traps for the next click on a door or container, Pick Pocket for the
//! next click on an NPC ([`crate::skill_use`], [`arcanum_sim::pickpocket`]). Prowling (sneaking)
//! and Repair are not built: their buttons do nothing yet. One view shows at a time (0x5506c0 toggles between them).
//!
//! Not built: the hover names (the original shows them in the view, 0x550860 / 0x5507e0;
//! the skills' are `mes/skill_ui.mes` 100-103); the other views.

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

/// The skills view's buttons (table 0x5c6c68 / 0x5cb220 / 0x5cb240): file, x, skill.
pub const SKILL_BUTTONS: [(&str, f32, usize); 4] = [
    (interface::SKILL_BUTTONS[0], 288.0, 6),
    (interface::SKILL_BUTTONS[1], 351.0, 5),
    (interface::SKILL_BUTTONS[2], 414.0, arcanum_sim::SKILL_DISARM_TRAPS),
    (interface::SKILL_BUTTONS[3], 477.0, 12),
];
pub const SKILL_Y: f32 = 514.0;
pub const SKILL_SIZE: f32 = 32.0;

/// Which view the bottom bar's centre shows.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BarView {
    /// The spells view instead of the message box.
    pub spells: bool,
    /// The skills view instead of the message box.
    pub skills: bool,
    /// The college whose spells show.
    pub college: Option<usize>,
}

/// Something drawn in the spells view.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpellViewPart {
    College(usize),
    Spell(usize),
    /// A skills view button, by its skill.
    Skill(usize),
}

/// The skills view button at `point`, by its skill.
pub fn skill_at(point: Vec2) -> Option<usize> {
    let inside = |x: f32| (x..x + SKILL_SIZE).contains(&point.x) && (SKILL_Y..SKILL_Y + SKILL_SIZE).contains(&point.y);
    SKILL_BUTTONS.iter().find(|b| inside(b.1)).map(|b| b.2)
}

pub struct SpellBarPlugin;

impl Plugin for SpellBarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BarView>()
            .init_resource::<SpellDrag>()
            .add_systems(OnExit(AppState::InGame), |mut view: ResMut<BarView>, mut drag: ResMut<SpellDrag>| {
                *view = BarView::default();
                *drag = SpellDrag::default();
            })
            .add_systems(Update, (close_above_the_bar, click_view, show_view, show_drag).chain().run_if(in_state(AppState::InGame)))
            .add_observer(on_spells_button);
    }
}

/// A spell's icon (`s_<college>n.art`, 0x4b1570).
pub fn spell_icon(spell: usize) -> String {
    let stem = crate::editor_tabs::COLLEGE_STEMS[(spell / SPELLS_PER_COLLEGE).min(COLLEGES - 1)];
    format!("{}/s_{stem}{}.art", interface::DIR, spell % SPELLS_PER_COLLEGE + 1)
}

/// The spell being dragged out of the view, if any.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpellDrag(pub Option<usize>);

/// The dragged spell's icon at the cursor.
#[derive(Component)]
pub struct DraggedSpell;

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

/// The spells and skills buttons toggle their view; one shows at a time (0x5506c0).
fn on_spells_button(activate: On<bevy::ui_widgets::Activate>, buttons: Query<&HudAction>, mut view: ResMut<BarView>) {
    match buttons.get(activate.entity) {
        Ok(HudAction::Spells) => (view.spells, view.skills) = (!view.spells, false),
        Ok(HudAction::Skills) => (view.skills, view.spells) = (!view.skills, false),
        _ => {}
    }
}

fn cursor(windows: &Query<&Window>) -> Option<Vec2> {
    windows.iter().find_map(|w| w.cursor_position().map(|c| screen_point(w.size(), c)))
}

/// The view closes when the cursor goes up past the bar's top (0x54b5d0); a drag ends.
fn close_above_the_bar(windows: Query<&Window>, mut view: ResMut<BarView>, mut drag: ResMut<SpellDrag>) {
    if (view.spells || view.skills) && cursor(&windows).is_some_and(|p| p.y < BOTTOM_TOP) {
        view.spells = false;
        view.skills = false;
        drag.0 = None;
    }
}

/// Press on a college shows its spells; press on a spell picks it up, and the release arms it
/// (on the spell) or puts it on a hotkey slot (0x57e8d0).
#[allow(clippy::too_many_arguments)]
fn click_view(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    mut view: ResMut<BarView>,
    mut drag: ResMut<SpellDrag>,
    sheet_host: Res<crate::pc_sheet::SheetHost>,
    mut inventory: ResMut<crate::inventory_screen::PcInventory>,
    armed: Option<ResMut<crate::skill_use::SkillTargeting>>,
    client: Option<Res<crate::world_render::ClientMap>>,
    (mut cache, assets, frames): (ResMut<crate::inventory_screen::ItemArtCache>, Res<AssetServer>, Res<Assets<crate::art_loader::ArtFrames>>),
) {
    let (pressed, released) = (buttons.just_pressed(MouseButton::Left), buttons.just_released(MouseButton::Left));
    // A skills view button arms its skill on release (Disarm Traps and Pick Pocket are built).
    if view.skills && released {
        if let Some(skill @ (arcanum_sim::SKILL_DISARM_TRAPS | arcanum_sim::SKILL_PICK_POCKET)) = cursor(&windows).and_then(skill_at)
            && let Some(mut armed) = armed
        {
            armed.0 = Some(crate::skill_use::Armed::Skill { skill, tool: None });
        }
        return;
    }
    if !view.spells || !(pressed || released) {
        return;
    }
    let (Some(point), Some(sheet)) = (cursor(&windows), sheet_host.sheet()) else { return };
    let part = part_at(point, &sheet.spells, view.college);
    if pressed {
        match part {
            Some(SpellViewPart::College(c)) => view.college = Some(c),
            Some(SpellViewPart::Spell(s)) => drag.0 = Some(s),
            Some(SpellViewPart::Skill(_)) | None => {}
        }
        return;
    }
    let Some(spell) = drag.0.take() else { return };
    if let Some(slot) = crate::hotkey_bar::slot_at(point) {
        let Some(inv) = inventory.shown().cloned() else { return };
        let empty = arcanum_formats::object::Prototypes::default();
        let prototypes = client.as_ref().map_or(&empty, |c| &c.map.prototypes);
        let sizes = crate::inventory_screen::inventory_sizes(&inv, prototypes, client.as_ref().and_then(|c| c.art_index()), &mut cache, &assets, &frames);
        let wearer = crate::inventory_screen::wearer(&sheet_host);
        let _ = inventory.request(arcanum_sim::InventoryRequest::HotkeySpell { spell, to: crate::hotkey_bar::location(slot) }, wearer, crate::inventory_screen::client_info(prototypes, &sizes));
    } else if part == Some(SpellViewPart::Spell(spell))
        && let Some(mut armed) = armed
    {
        armed.0 = Some(crate::skill_use::Armed::Spell(spell));
    }
}

/// Draws the dragged spell's icon with its top-left at the cursor.
fn show_drag(mut commands: Commands, drag: Res<SpellDrag>, windows: Query<&Window>, assets: Res<AssetServer>, boxes: Query<&ChildOf, With<MessageBoxRoot>>, mut icons: Query<(Entity, &mut Node), With<DraggedSpell>>) {
    let (Some(spell), Some(point)) = (drag.0, cursor(&windows)) else {
        for (e, _) in &icons {
            commands.entity(e).despawn();
        }
        return;
    };
    let corner = Vec2::new(point.x, point.y - BOTTOM_TOP);
    if let Ok((_, mut node)) = icons.single_mut() {
        node.left = crate::menu_window::px(corner.x);
        node.top = crate::menu_window::px(corner.y);
        return;
    }
    let Ok(bar) = boxes.single() else { return };
    let icon = commands.spawn((DraggedSpell, bevy::picking::Pickable::IGNORE, GlobalZIndex(10), ImageNode::new(assets.load(spell_icon(spell))), at(rect(corner, Vec2::splat(SPELL_SIZE))))).id();
    commands.entity(bar.parent()).add_child(icon);
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
    *visibility = if view.spells || view.skills { Visibility::Hidden } else { Visibility::Inherited };
    let local = |x: f32, y: f32, size: f32| at(rect(Vec2::new(x, y - BOTTOM_TOP), Vec2::splat(size)));
    let mut spawned = Vec::new();
    if view.skills {
        for (file, x, skill) in SKILL_BUTTONS {
            spawned.push(commands.spawn((SpellViewPart::Skill(skill), bevy::picking::Pickable::IGNORE, ImageNode::new(assets.load(file)), local(x, SKILL_Y, SKILL_SIZE))).id());
        }
    }
    if !view.spells {
        commands.entity(parent.parent()).add_children(&spawned);
        return;
    }
    for (c, path) in crate::editor_tabs::college_buttons().into_iter().enumerate().filter(|(c, _)| spells[*c] > 0) {
        spawned.push(commands.spawn((SpellViewPart::College(c), bevy::picking::Pickable::IGNORE, ImageNode::new(assets.load(path)), local(COLLEGE_X[c], COLLEGE_Y, COLLEGE_SIZE))).id());
    }
    if let Some(college) = view.college {
        for (spell, x) in shown_spells(college, spells[college]).zip(SPELL_X) {
            spawned.push(commands.spawn((SpellViewPart::Spell(spell), bevy::picking::Pickable::IGNORE, ImageNode::new(assets.load(spell_icon(spell))), local(x, SPELL_Y, SPELL_SIZE))).id());
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

    #[test]
    fn the_skills_view_has_four_buttons() {
        assert_eq!(skill_at(Vec2::new(290.0, 520.0)), Some(6), "Prowling");
        assert_eq!(skill_at(Vec2::new(420.0, 530.0)), Some(arcanum_sim::SKILL_DISARM_TRAPS));
        assert_eq!(skill_at(Vec2::new(508.0, 545.0)), Some(12), "Repair");
        assert_eq!(skill_at(Vec2::new(330.0, 520.0)), None, "between");
    }
}
