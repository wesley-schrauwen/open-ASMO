//! Using a skill or casting a spell on something in the world: for now Pick Locks
//! ([`arcanum_sim::lockpick`]) and the Unlocking Cantrip ([`arcanum_sim::magick`]) on a door or
//! container.
//!
//! - **Arming:** releasing a lockpick on the inventory screen's use box closes the screen and
//!   arms Pick Locks with it as the tool (the original's use box; also its hotkey bar, not built).
//! - A spell is armed from the bottom bar's spells view ([`crate::spell_bar`]).
//! - While [`SkillTargeting`] is armed, a left click on a door or container walks the PC to it
//!   and asks the host to pick it, or casts the spell on it from where the PC stands (0x57c110);
//!   a click elsewhere, a right click or Escape puts it away (0x57bff0). The host's answer
//!   (`mes/skill_ui.mes`, `mes/spell.mes`) shows in the message box.

use bevy::prelude::*;

use crate::main_menu::AppState;

/// The skill the next click on the world uses, if any.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SkillTargeting(pub Option<Armed>);

/// What the next click uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Armed {
    /// A skill, with the tool used (its inventory location).
    Skill { skill: usize, tool: Option<i32> },
    /// A spell.
    Spell(usize),
}

/// A host's answer: a `mes/skill_ui.mes` or `mes/spell.mes` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseResult {
    Skill(u32),
    Spell(u32),
}

/// The host's answers to skill uses and casts, waiting for the message box.
#[derive(Resource, Debug, Default)]
pub struct SkillResults(pub Vec<UseResult>);

pub struct SkillUsePlugin;

impl Plugin for SkillUsePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SkillTargeting>()
            .init_resource::<SkillResults>()
            .add_systems(Update, (cancel_targeting, show_results).run_if(in_state(AppState::InGame)));
    }
}

/// A right click or Escape puts the skill away.
fn cancel_targeting(mut targeting: ResMut<SkillTargeting>, buttons: Res<ButtonInput<MouseButton>>, mut keys: ResMut<ButtonInput<KeyCode>>) {
    if targeting.0.is_none() {
        return;
    }
    if buttons.just_pressed(MouseButton::Right) || keys.clear_just_pressed(KeyCode::Escape) {
        targeting.0 = None;
    }
}

/// Posts the host's answers in the message box.
fn show_results(mut results: ResMut<SkillResults>, data: Option<Res<crate::create_character::CreationData>>, boxed: Option<ResMut<crate::message_box::MessageBox>>, time: Option<Res<Time<Real>>>) {
    if results.0.is_empty() {
        return;
    }
    let lines = std::mem::take(&mut results.0);
    let Some(mut boxed) = boxed else { return };
    for line in lines {
        let text = data
            .as_ref()
            .and_then(|d| match line {
                UseResult::Skill(n) => d.skill_ui.get(n),
                UseResult::Spell(n) => d.spell.get(n),
            })
            .unwrap_or("")
            .to_string();
        boxed.post(crate::message_box::BoxMessage::plain(text), time.as_ref().map_or(0.0, |t| t.elapsed_secs_f64()));
    }
}
