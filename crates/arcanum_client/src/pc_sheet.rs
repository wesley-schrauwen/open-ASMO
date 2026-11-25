//! The client's copy of its own PC's sheet in the world, for the in-game character screen
//! ([`arcanum_sim::sheet`]).
//!
//! As with character creation ([`crate::character_host`]): a change is applied at once to a
//! preview with the host's rules and sent as a numbered request. The host answers each with
//! its copy; the preview becomes that copy with the unanswered requests replayed on top.

use std::collections::VecDeque;

use arcanum_rules::{CharacterSheet, Refusal, RulesData};
use arcanum_sim::{LedgerDelta, PlayerIntent, SheetRequest, apply_sheet_request};
use bevy::prelude::*;

#[derive(Resource, Debug, Clone, Default)]
pub struct SheetHost {
    /// The host's copy, once it has sent one (with the PC's spawn).
    confirmed: Option<CharacterSheet>,
    /// `confirmed` with `pending` applied: what the screen shows.
    preview: Option<CharacterSheet>,
    pending: VecDeque<(u32, SheetRequest)>,
    last_seq: u32,
    outgoing: Vec<PlayerIntent>,
}

impl SheetHost {
    /// The sheet to show, once the host has sent it.
    pub fn sheet(&self) -> Option<&CharacterSheet> {
        self.preview.as_ref()
    }

    /// Applies `request` to the preview and queues it for the host. Refused requests change
    /// nothing and are not sent; nothing is sent before the host's copy is in.
    pub fn request(&mut self, rules: &RulesData, request: SheetRequest) -> Result<(), Refusal> {
        let Some(preview) = &self.preview else {
            return Err(Refusal::NotAllowed);
        };
        let changed = apply_sheet_request(rules, preview, &request)?;
        self.preview = Some(changed);
        self.last_seq += 1;
        self.pending.push_back((self.last_seq, request.clone()));
        self.outgoing.push(PlayerIntent::Sheet {
            seq: self.last_seq,
            request,
        });
        Ok(())
    }

    /// Requests for the host, oldest first.
    pub fn take_outgoing(&mut self) -> Vec<PlayerIntent> {
        std::mem::take(&mut self.outgoing)
    }

    /// Takes the host's copy from a `SheetUpdate`; other deltas are ignored.
    pub fn receive(&mut self, rules: &RulesData, delta: &LedgerDelta) {
        let LedgerDelta::SheetUpdate { ack, sheet } = delta else {
            return;
        };
        while self.pending.front().is_some_and(|(seq, _)| seq <= ack) {
            self.pending.pop_front();
        }
        let mut preview = sheet.clone();
        for (_, request) in &self.pending {
            if let Ok(changed) = apply_sheet_request(rules, &preview, request) {
                preview = changed;
            }
        }
        self.confirmed = Some(sheet.clone());
        self.preview = Some(preview);
    }
}

#[cfg(test)]
mod tests {
    use arcanum_rules::{CharacterAction, Pool};

    use super::*;

    fn sheet(points: i32) -> CharacterSheet {
        CharacterSheet {
            unspent: points,
            ..RulesData::default().new_sheet()
        }
    }

    #[test]
    fn nothing_is_sent_before_the_hosts_copy() {
        let rules = RulesData::default();
        let mut host = SheetHost::default();
        assert!(host.request(&rules, SheetRequest::Open).is_err());
        assert!(host.take_outgoing().is_empty() && host.sheet().is_none());
    }

    #[test]
    fn changes_show_at_once_and_the_hosts_answers_win() {
        let rules = RulesData::default();
        let mut host = SheetHost::default();
        host.receive(
            &rules,
            &LedgerDelta::SheetUpdate {
                ack: 0,
                sheet: sheet(2),
            },
        );
        host.request(&rules, SheetRequest::Open).unwrap();
        host.request(
            &rules,
            SheetRequest::Action(CharacterAction::AdjustPool(Pool::Fatigue, 1)),
        )
        .unwrap();
        assert_eq!(host.sheet().unwrap().draft.fatigue, 1, "at once");
        let sent = host.take_outgoing();
        assert_eq!(sent.len(), 2);
        assert!(matches!(&sent[1], PlayerIntent::Sheet { seq: 2, .. }));
        // The host answered the Open with a sheet that has 3 points: the fatigue point is
        // replayed on it.
        host.receive(
            &rules,
            &LedgerDelta::SheetUpdate {
                ack: 1,
                sheet: sheet(3),
            },
        );
        assert_eq!(
            (
                host.sheet().unwrap().draft.fatigue,
                rules.unspent(host.sheet().unwrap())
            ),
            (1, 2)
        );
        // The answer to it leaves nothing to replay.
        let answered = host.sheet().unwrap().clone();
        host.receive(
            &rules,
            &LedgerDelta::SheetUpdate {
                ack: 2,
                sheet: answered.clone(),
            },
        );
        assert_eq!(host.sheet(), Some(&answered));
        assert_eq!(
            host.request(
                &rules,
                SheetRequest::Action(CharacterAction::SetName("X".into()))
            ),
            Err(Refusal::NotAllowed)
        );
    }
}
