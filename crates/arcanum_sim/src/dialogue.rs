//! Conversations with NPCs, run by the host (Arcanum.exe: talk 0x42af00 → 0x57cc70, start
//! 0x567460, the dialogue module 0x412e10-0x419e70). See `plans/dialogue.md`.
//!
//! - **Talking** ([`PlayerIntent::Talk`](crate::PlayerIntent::Talk)): the PC walks to the NPC;
//!   within [`crate::USE_RANGE`] the NPC's dialogue script (event [`EVENT_DIALOGUE`]) runs. Its
//!   action 4 starts a conversation in the script's own dialogue file ([`StartedDialogues`]);
//!   any script can start one the same way.
//! - **A line** (0x416840): the NPC's text for the PC's gender, names put in (`@pcname@`,
//!   `@npcname@`), and its result run. It floats over the NPC for every player
//!   ([`LedgerDelta::FloatText`](crate::LedgerDelta::FloatText)); its voice file goes with it.
//! - **Options** (0x414f50): the PC options after the line up to the next NPC line that pass
//!   gender, intelligence and the test; at most [`MAX_OPTIONS`]. Only the talker gets them
//!   ([`LedgerDelta::DialogueOptions`](crate::LedgerDelta::DialogueOptions)). None: the
//!   conversation ends.
//! - **Choosing** ([`PlayerIntent::Choose`](crate::PlayerIntent::Choose)): the option floats
//!   over the PC (green), its result runs, then its response: a line, 0 the end, `-n` line `n`
//!   (an NPC line is shown; a PC option is taken as chosen, without showing it: builder's
//!   reading of the undecoded state 2).
//! - **Ending** ([`PlayerIntent::EndTalk`](crate::PlayerIntent::EndTalk), the response 0, no
//!   options): the talker gets [`LedgerDelta::DialogueEnded`](crate::LedgerDelta::DialogueEnded);
//!   the NPC has now met the PC (test `me`).
//!
//! **Built tests:** `gv gf pv pf lf lc qu qa qb ss ru rq rp me $$ in ni ch pe le ra ps ha`.
//! **Built results:** `$$ gv gf pv pf lf lc qu ss ru rq rp fl`. The NPC's flags and counters
//! (`lf`, `lc`) are its dialogue attachment's (event 9, "scope 9"). Item numbers are basic
//! prototype numbers (+20, as `InvenSource`; to confirm). A test the host cannot check yet fails;
//! a result it cannot do yet is skipped; both are reported ([`DialogueReports`]).
//!
//! **Generic option texts** (`Y: N: S: E: F: K: W:`) are a random line of the range from
//! `mes/gd_pc2m.mes` or `gd_pc2f.mes` by the NPC's gender (the `gd_dumb_` pair for a PC of
//! Intelligence 4 or less). Other prefixed options, and `G:` NPC lines, are not built: the option
//! is left out, a `G:` line ends the conversation.
//!
//! Not built: barks for NPCs without a script (0x5681c0); the NPC's can-talk checks (0x4ad800);
//! reaction, alignment, aptitude, training and the rest of the tests and results; turning to face
//! each other.

use std::collections::{BTreeSet, HashMap};

use arcanum_formats::dialog::{DialogFile, DialogLine, Token, effects, tests};
use arcanum_formats::mes::MesFile;
use arcanum_rules::{CharacterSheet, FEMALE, RulesData};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{Attachment, Inventory, LedgerDelta, LoadedMap, NetId, Outbox, PcScriptState, PlayerControlled, PlayerId, Position, ScriptGlobals, ScriptObject, Session, Sheet, SimRules, WorldState};

/// The dialogue event (attach point 9).
pub const EVENT_DIALOGUE: u32 = 9;
/// Most options shown.
pub const MAX_OPTIONS: usize = arcanum_formats::dialog::MAX_OPTIONS;
/// How far `-n` responses may chain.
const MAX_JUMPS: usize = 8;

/// Which map object an NPC is: its place in its sector's mobile objects.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct NpcSource {
    pub sector: arcanum_formats::map::SectorId,
    pub index: u32,
}

impl NpcSource {
    pub fn object(self) -> ScriptObject {
        ScriptObject::Mobile { sector: self.sector, index: self.index }
    }
}

/// A conversation a script started: the talker, the other party, the file and the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartedDialogue {
    pub player: PlayerId,
    pub npc: ScriptObject,
    pub dialog: u32,
    pub line: i32,
}

/// Conversations started this tick, waiting for [`run_conversations`].
#[derive(Resource, Debug, Default)]
pub struct StartedDialogues(pub Vec<StartedDialogue>);

/// A PC's conversation: with whom, in which file, the NPC line shown and the options offered
/// (their lines and texts).
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub npc: NetId,
    pub npc_object: ScriptObject,
    pub dialog: u32,
    pub line: i32,
    pub options: Vec<(i32, String)>,
}

/// A walk to an NPC to talk to it; `seq` is the move it came with.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingTalk {
    pub npc: NetId,
    pub seq: u32,
}

/// Requests taken from the inbox.
#[derive(Resource, Debug, Default)]
pub struct TalkRequests {
    pub talk: Vec<(PlayerId, NetId, u32)>,
    pub choose: Vec<(PlayerId, usize)>,
    pub end: Vec<PlayerId>,
}

/// The reputation effects on reaction, read on first use.
#[derive(Resource, Debug, Default)]
pub struct ReputationTable(pub Option<crate::ReputationEffects>);

/// Tests and results the host met but cannot do yet: the dialogue file and the opcode.
#[derive(Resource, Debug, Default)]
pub struct DialogueReports(pub Vec<(u32, String)>);

/// The generic line files and NPC names, read on first use.
#[derive(Resource, Debug, Default)]
pub struct GenericLines(HashMap<String, Option<MesFile>>);

impl GenericLines {
    fn file(&mut self, vfs: Option<&arcanum_vfs::Vfs>, path: &str) -> Option<&MesFile> {
        self.0.entry(path.to_string()).or_insert_with(|| vfs.and_then(|v| v.read(path).ok()).and_then(|d| MesFile::parse(&d).ok())).as_ref()
    }
}

/// How a line floats (0x4d5fe0's colours).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FloatColour {
    /// An NPC's line.
    White,
    /// The PC's chosen line.
    Green,
}

/// `n` as a threshold: `n >= 0` at least `n`, `n < 0` at most `-n`.
pub fn passes(value: i32, n: i32) -> bool {
    if n < 0 { value <= -n } else { value >= n }
}

/// What tests and results read and change for one PC talking to one NPC.
pub struct Talk<'a> {
    pub rules: &'a RulesData,
    pub sheet: &'a CharacterSheet,
    pub inventory: &'a mut Inventory,
    pub pc: &'a mut PcScriptState,
    pub globals: &'a mut ScriptGlobals,
    /// The NPC's dialogue attachment (flags and counters).
    pub npc: &'a mut Attachment,
    pub met: bool,
    /// The NPC's key in the PC's stored reactions, and what its reaction is made of.
    pub npc_key: String,
    pub npc_reaction: crate::NpcReaction,
    pub reputations: &'a crate::ReputationEffects,
    /// Gold changed: the inventory must be sent.
    pub gold_changed: bool,
    /// Opcodes not built yet that were met.
    pub unbuilt: Vec<String>,
}

impl Talk<'_> {
    pub fn gold(&self) -> i32 {
        self.inventory.items.iter().filter(|i| i.prototype == crate::GOLD).map(|i| i.quantity).sum()
    }

    fn has(&self, basic: i32) -> bool {
        let prototype = basic as u32 + crate::BASIC_TO_PROTOTYPE;
        self.inventory.items.iter().any(|i| i.prototype == prototype)
    }

    /// Gives (`n > 0`) or takes (`n < 0`) gold; a PC with no gold yet gets a stack next to
    /// its first item (gold takes no grid room here; builder's call).
    fn change_gold(&mut self, n: i32) {
        if n == 0 {
            return;
        }
        let stack = self.inventory.items.iter_mut().find(|i| i.prototype == crate::GOLD);
        match stack {
            Some(stack) => stack.quantity = (stack.quantity + n).max(0),
            None if n > 0 => self.inventory.items.insert(0, crate::CarriedItem { prototype: crate::GOLD, location: 0, quantity: n, keys: Vec::new() }),
            None => return,
        }
        self.inventory.items.retain(|i| i.prototype != crate::GOLD || i.quantity > 0);
        self.gold_changed = true;
    }

    fn stat(&self, index: usize) -> i32 {
        self.rules.stats(self.sheet).get(index).copied().unwrap_or(0)
    }

    /// The NPC's reaction toward the PC (0x4c0d00): stored, else its base, plus the modifier.
    pub fn reaction(&self) -> i32 {
        self.stored_reaction() + self.reaction_modifier()
    }

    fn stored_reaction(&self) -> i32 {
        self.pc.reactions.get(&self.npc_key).copied().unwrap_or(self.npc_reaction.base)
    }

    fn reaction_modifier(&self) -> i32 {
        let reputations = self.reputations.on(self.pc.reputations.iter().copied(), self.npc_reaction.origin, self.npc_reaction.faction);
        self.npc_reaction.modifier(self.rules.derived(self.sheet).reaction, self.sheet.race, reputations)
    }

    /// Changes the reaction (dialogue `re`, 0x4c0de0): adds, or brings it to, at least or at most
    /// `n`; the change is stored for this PC.
    fn change_reaction(&mut self, n: i32, change: arcanum_formats::dialog::Change) {
        use arcanum_formats::dialog::Change;
        let now = self.reaction();
        let delta = match change {
            Change::Add => n,
            Change::Set => n - now,
            Change::AtLeast => (n - now).max(0),
            Change::AtMost => (n - now).min(0),
        };
        if delta != 0 {
            let stored = self.stored_reaction() + delta;
            self.pc.reactions.insert(self.npc_key.clone(), stored);
        }
    }

    pub fn female(&self) -> bool {
        self.sheet.gender == FEMALE
    }

    pub fn intelligence(&self) -> i32 {
        self.stat(4)
    }

    /// One test token (0x4150d0); `None` when the host cannot check it yet.
    fn test(&mut self, t: &Token) -> Option<bool> {
        let (a, b) = (t.a, t.b);
        Some(match t.op() {
            "gv" => self.globals.var(a) == b,
            "gf" => i32::from(self.globals.flag(a)) == b,
            "pv" => self.pc.vars.get(&(a as u32)).copied().unwrap_or(0) == b,
            "pf" => i32::from(self.pc.flags.contains(&(a as u32))) == b,
            "lf" => i32::from(self.npc.flags >> (a & 31) & 1 != 0) == b,
            "lc" => usize::try_from(a).ok().and_then(|i| self.npc.counters.get(i)).map_or(0, |&c| i32::from(c)) == b,
            "qu" => self.pc.quest(a) == b,
            "qa" => self.pc.quest(a) >= b,
            "qb" => self.pc.quest(a) <= b,
            "ss" => passes(self.globals.story_state, a),
            "ru" => self.pc.rumours.contains(&a.unsigned_abs()) == (a >= 0),
            "rq" => self.globals.quelled.contains(&a.unsigned_abs()) == (a >= 0),
            "rp" => self.pc.reputations.contains(&a.unsigned_abs()) == (a > 0),
            "me" => (self.met || self.pc.reactions.contains_key(&self.npc_key)) == (a != 0),
            "re" => passes(self.reaction(), a),
            "$$" => passes(self.gold(), a),
            "in" if a > 0 => self.has(a),
            "ni" if a > 0 => !self.has(a),
            "ch" => passes(self.stat(7), a),
            "pe" => passes(self.stat(5), a),
            "le" => passes(self.sheet.level, a),
            "ra" => (i32::from(self.sheet.race) + 1 == a.abs()) == (a > 0),
            "ps" => passes(self.rules.skill_level(self.sheet, 11), a),
            "ha" => passes(self.rules.skill_level(self.sheet, 9), a),
            _ => return None,
        })
    }

    /// Whether a test passes: every token must (an unknown opcode fails it).
    pub fn passes_test(&mut self, text: &str) -> bool {
        let Ok(tokens) = tests(text) else { return false };
        tokens.iter().all(|t| match self.test(t) {
            Some(passed) => passed,
            None => {
                self.unbuilt.push(format!("test {}", t.op()));
                false
            }
        })
    }

    /// Runs a result (0x415ba0) left to right; an unknown opcode stops it. Returns the line `fl`
    /// closes with, if any (it stops the result too).
    pub fn run_result(&mut self, text: &str) -> Option<i32> {
        let tokens = match effects(text) {
            Ok(t) => t,
            Err(e) => e.before,
        };
        for t in &tokens {
            let (a, b) = (t.a, t.b);
            match t.op() {
                "$$" => self.change_gold(a),
                "gv" => self.globals.set_var(a, b),
                "gf" => self.globals.set_flag(a, b != 0),
                "pv" => set_or_remove(&mut self.pc.vars, a as u32, b),
                "pf" => set_in(&mut self.pc.flags, a as u32, b != 0),
                "lf" => {
                    let bit = 1 << (a & 31);
                    self.npc.flags = if b != 0 { self.npc.flags | bit } else { self.npc.flags & !bit };
                }
                "lc" => {
                    if let Some(c) = usize::try_from(a).ok().and_then(|i| self.npc.counters.get_mut(i)) {
                        *c = b as u8;
                    }
                }
                "qu" => set_or_remove(&mut self.pc.quests, a as u32, b),
                "ss" => self.globals.story_state = self.globals.story_state.max(a),
                "ru" => set_in(&mut self.pc.rumours, a.unsigned_abs(), true),
                "rq" => set_in(&mut self.globals.quelled, a.unsigned_abs(), true),
                "rp" => set_in(&mut self.pc.reputations, a.unsigned_abs(), a > 0),
                "fl" => return Some(a),
                "re" => self.change_reaction(a, t.change),
                op => self.unbuilt.push(format!("result {op}")),
            }
        }
        None
    }
}

fn set_or_remove(map: &mut std::collections::BTreeMap<u32, i32>, key: u32, value: i32) {
    if value == 0 {
        map.remove(&key);
    } else {
        map.insert(key, value);
    }
}

fn set_in(set: &mut BTreeSet<u32>, key: u32, on: bool) {
    if on {
        set.insert(key);
    } else {
        set.remove(&key);
    }
}

/// A PC option's generic range (0x416c10), for the prefixes built.
pub fn generic_range(text: &str) -> Option<std::ops::Range<u32>> {
    Some(match text.trim() {
        "Y:" | "y:" => 1..100,
        "N:" | "n:" => 100..200,
        "S:" | "s:" => 200..300,
        "E:" | "e:" => 400..500,
        "F:" | "f:" => 800..900,
        "K:" | "k:" => 1500..1600,
        "W:" | "w:" => 1800..1900,
        _ => return None,
    })
}

/// Whether an option's text is a generic prefix (`X:` or `X:args`).
pub fn is_prefixed(text: &str) -> bool {
    let b = text.trim_start().as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// The options after NPC line `line` that `talk` may choose (0x414f50): the PC options up to the
/// next NPC line, by gender, intelligence and test, at most [`MAX_OPTIONS`].
pub fn options(file: &DialogFile, line: i32, talk: &mut Talk) -> Vec<(i32, DialogLine)> {
    let mut out = Vec::new();
    let iq = talk.intelligence();
    for (&n, entry) in file.lines.range(line + 1..) {
        if entry.is_npc() || out.len() == MAX_OPTIONS {
            break;
        }
        if entry.for_males().is_some_and(|males| males == talk.female()) {
            continue;
        }
        if !passes(iq, entry.intelligence()) {
            continue;
        }
        if !entry.test.trim().is_empty() && !talk.passes_test(&entry.test) {
            continue;
        }
        out.push((n, entry.clone()));
    }
    out
}

/// Puts the names in (0x416b00): `@pcname@` and `@npcname@`, any case.
pub fn with_names(text: &str, pc: &str, npc: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let lower = tail.to_ascii_lowercase();
        if lower.starts_with("@pcname@") {
            out.push_str(pc);
            rest = &tail[8..];
        } else if lower.starts_with("@npcname@") {
            out.push_str(npc);
            rest = &tail[9..];
        } else {
            out.push('@');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// The voice file of line `voice` of dialogue `dialog` for a PC of that gender (0x5688d0;
/// builder's call: the PC's gender, as the line's text). The client falls back to `m`.
pub fn voice_path(dialog: u32, voice: i32, female: bool) -> Option<String> {
    (voice > 0 && dialog > 0).then(|| format!("sound/speech/{:05}/v{}_{}.mp3", dialog & 0x7fff, voice, if female { 'f' } else { 'm' }))
}

type WalkerQuery<'a> = (Entity, &'a PlayerControlled, &'a Position, Option<&'a PendingTalk>, Has<crate::Walking>, Has<Conversation>);
type TalkerQuery<'a> = (Entity, &'a NetId, &'a PlayerControlled, &'a Position, &'a Sheet, &'a mut Inventory, Option<&'a mut Conversation>, Option<&'a PendingTalk>, Has<crate::Walking>);

/// Walks PCs to the NPCs they want to talk to and runs those NPCs' dialogue scripts.
#[allow(clippy::too_many_arguments)]
pub fn talk_to_npcs(
    mut commands: Commands,
    mut requests: ResMut<TalkRequests>,
    mut world: ResMut<WorldState>,
    mut map: ResMut<LoadedMap>,
    mut outbox: ResMut<Outbox>,
    mut scripts: crate::ScriptHost,
    pcs: Query<WalkerQuery>,
    npcs: Query<(&NetId, &NpcSource, &Position)>,
) {
    let mut fresh: HashMap<Entity, PendingTalk> = HashMap::new();
    for (player, npc, seq) in requests.talk.drain(..) {
        if let Some((entity, ..)) = pcs.iter().find(|pc| pc.1.0 == player) {
            fresh.insert(entity, PendingTalk { npc, seq });
        }
    }
    for (entity, player, position, pending, walking, talking) in &pcs {
        let Some(want) = fresh.get(&entity).copied().or(pending.copied()) else { continue };
        let Some((_, source, at)) = npcs.iter().find(|n| *n.0 == want.npc) else {
            commands.entity(entity).remove::<PendingTalk>();
            continue;
        };
        if !crate::within_radius(position.0, at.0, crate::USE_RANGE) {
            if walking {
                commands.entity(entity).insert(want);
            } else {
                commands.entity(entity).remove::<PendingTalk>();
            }
            continue;
        }
        commands.entity(entity).remove::<PendingTalk>();
        if talking {
            continue;
        }
        // The NPC's dialogue script (0x57cc70); its action 4 starts the conversation.
        scripts.hook(&mut world, &mut map, &mut outbox, source.object(), EVENT_DIALOGUE, player.0);
    }
}

/// Starts, steps and ends conversations: see the module docs.
#[allow(clippy::too_many_arguments)]
pub fn run_conversations(
    mut commands: Commands,
    mut started: ResMut<crate::StartedDialogues>,
    mut requests: ResMut<TalkRequests>,
    mut world: ResMut<WorldState>,
    mut map: ResMut<LoadedMap>,
    mut scripts: ResMut<crate::Scripts>,
    mut generic: ResMut<GenericLines>,
    mut reports: ResMut<DialogueReports>,
    (session, rules, vfs, acks): (Res<Session>, Res<SimRules>, Option<Res<crate::SimVfs>>, Res<crate::InventoryAcks>),
    (mut rng, mut reputations): (ResMut<crate::HostRng>, ResMut<ReputationTable>),
    mut outbox: ResMut<Outbox>,
    mut pcs: Query<TalkerQuery>,
    npcs: Query<(&NetId, &NpcSource)>,
) {
    if started.0.is_empty() && requests.choose.is_empty() && requests.end.is_empty() {
        return;
    }
    let vfs_guard = vfs.as_ref().and_then(|v| v.0.read().ok());
    let vfs = vfs_guard.as_deref();
    let library = scripts.0.get_or_insert_with(|| vfs.map(crate::ScriptLibrary::new).unwrap_or_default());
    let reputations = &*reputations.0.get_or_insert_with(|| {
        vfs.and_then(|v| v.read(crate::GAME_REP).ok()).and_then(|d| MesFile::parse(&d).ok()).map(|m| crate::ReputationEffects::parse(&m)).unwrap_or_default()
    });
    let mut steps: Vec<(PlayerId, Step)> = started.0.drain(..).map(|s| (s.player, Step::Start(s))).collect();
    steps.extend(requests.choose.drain(..).map(|(p, i)| (p, Step::Choose(i))));
    steps.extend(requests.end.drain(..).map(|p| (p, Step::End)));
    for (player, step) in steps {
        let Some((entity, &pc_net, _, _, sheet, mut inventory, conversation, ..)) = pcs.iter_mut().find(|pc| pc.2.0 == player) else { continue };
        let Some(save) = world.0.as_mut() else { continue };
        let Some(saved) = save.players.get_mut(&player) else { continue };
        let (npc_object, dialog, current) = match (&step, conversation.as_deref()) {
            (Step::Start(s), _) => (s.npc, s.dialog, None),
            (_, Some(c)) => (c.npc_object, c.dialog, Some(c.clone())),
            (_, None) => continue,
        };
        let npc_net = match npc_object {
            ScriptObject::Mobile { sector, index } => npcs.iter().find(|n| n.1.sector == sector && n.1.index == index).map(|n| *n.0),
            _ => None,
        };
        let Some(npc_net) = npc_net else { continue };
        let Some(file) = vfs.and_then(|v| library.dialog(v, dialog)) else { continue };
        let key = met_key(npc_object);
        let mut npc_attachment = map.attachment(npc_object, EVENT_DIALOGUE);
        let before = npc_attachment;
        let npc_info = npc_details(&map, npc_object, &mut generic, vfs);
        let met = saved.script_state.met.contains(&key);
        let mut talk = Talk {
            rules: &rules.0,
            sheet: &sheet.0,
            inventory: &mut inventory,
            pc: &mut saved.script_state,
            globals: &mut save.scripts,
            npc: &mut npc_attachment,
            met,
            npc_key: key.clone(),
            npc_reaction: npc_info.reaction,
            reputations,
            gold_changed: false,
            unbuilt: Vec::new(),
        };
        let mut say = Say { npc_net, dialog, pc_name: &sheet.0.name, npc: &npc_info, generic: &mut generic, vfs, rng: &mut rng, deltas: Vec::new() };
        let next = match step {
            Step::Start(s) => say.show(&file, s.line, &mut talk),
            Step::Choose(i) => match current.as_ref().and_then(|c| c.options.get(i)) {
                Some((line, text)) => {
                    say.float(pc_net, text.clone(), FloatColour::Green, None);
                    say.chosen(&file, *line, &mut talk, 0)
                }
                None => Shown::Stay,
            },
            Step::End => Shown::End,
        };
        let gold_changed = talk.gold_changed;
        for op in talk.unbuilt.drain(..) {
            reports.0.push((dialog, op));
        }
        let excess = reports.0.len().saturating_sub(crate::REPORTS_KEPT);
        reports.0.drain(..excess);
        let deltas = std::mem::take(&mut say.deltas);
        let mut ended = false;
        match next {
            Shown::Line(line, options) => {
                let texts = options.iter().map(|(_, t)| t.clone()).collect();
                commands.entity(entity).insert(Conversation { npc: npc_net, npc_object, dialog, line, options });
                outbox.0.push((player, LedgerDelta::DialogueOptions { npc: npc_net, options: texts }));
            }
            Shown::End => ended = true,
            Shown::Stay => {}
        }
        if ended {
            commands.entity(entity).remove::<Conversation>();
            saved.script_state.met.insert(key);
            outbox.0.push((player, LedgerDelta::DialogueEnded));
        }
        if gold_changed {
            saved.inventory = Some(inventory.clone());
            let ack = acks.0.get(&player).copied().unwrap_or(0);
            outbox.0.push((player, LedgerDelta::Inventory { net_id: pc_net, ack, inventory: inventory.clone() }));
        }
        for delta in deltas {
            crate::ground::broadcast(&session, &mut outbox, delta);
        }
        if npc_attachment != before {
            map.set_attachment(npc_object, EVENT_DIALOGUE, npc_attachment);
            if let ScriptObject::Mobile { sector, .. } = npc_object {
                crate::ground::save_change(&mut world, &map, sector);
            }
        }
    }
}

enum Step {
    Start(StartedDialogue),
    Choose(usize),
    End,
}

/// What follows a step.
enum Shown {
    /// An NPC line with these options (line, text).
    Line(i32, Vec<(i32, String)>),
    End,
    /// Nothing changes (a choice that is not there).
    Stay,
}

/// The NPC's name and gender.
struct NpcInfo {
    name: String,
    male: bool,
    reaction: crate::NpcReaction,
}

fn met_key(npc: ScriptObject) -> String {
    match npc {
        ScriptObject::Mobile { sector, index } => format!("{}:{}", sector.0, index),
        ScriptObject::Map(id) => format!("map {}:{}", id.sector.0, id.index),
        ScriptObject::Pc(p) => format!("pc {}", p.0),
    }
}

/// The NPC's name (`mes/description.mes` by its `description`) and gender (stat 26, 1 male).
fn npc_details(map: &LoadedMap, npc: ScriptObject, generic: &mut GenericLines, vfs: Option<&arcanum_vfs::Vfs>) -> NpcInfo {
    let object = match npc {
        ScriptObject::Mobile { sector, index } => map.mobiles.get(&sector).and_then(|m| m.get(index as usize)),
        ScriptObject::Map(id) => map.map_object(id),
        ScriptObject::Pc(_) => None,
    };
    let none = crate::NpcReaction { base: crate::BASE_REACTION, race: 0, origin: 0, faction: 0, only_reputation: false };
    let Some(object) = object else { return NpcInfo { name: String::new(), male: true, reaction: none } };
    let description = map.prototypes.int(object, "description");
    let name = description.and_then(|d| generic.file(vfs, "mes/description.mes")?.get(d as u32).map(str::to_string)).unwrap_or_default();
    let stats = object.ints("critter_stat_base_idx").or_else(|| map.prototypes.of(object)?.ints("critter_stat_base_idx"));
    let male = stats.and_then(|s| s.get(26).copied()).is_none_or(|g| g == 1);
    NpcInfo { name, male, reaction: crate::NpcReaction::of(object, &map.prototypes) }
}

/// What one step says, to whom.
struct Say<'a> {
    npc_net: NetId,
    dialog: u32,
    pc_name: &'a str,
    npc: &'a NpcInfo,
    generic: &'a mut GenericLines,
    vfs: Option<&'a arcanum_vfs::Vfs>,
    rng: &'a mut crate::HostRng,
    deltas: Vec<LedgerDelta>,
}

impl Say<'_> {
    fn float(&mut self, speaker: NetId, text: String, colour: FloatColour, voice: Option<String>) {
        self.deltas.push(LedgerDelta::FloatText { speaker, text, colour, voice });
    }

    /// Shows NPC line `line` and offers its options (0x414e60).
    fn show(&mut self, file: &DialogFile, line: i32, talk: &mut Talk) -> Shown {
        let Some(entry) = file.lines.get(&line).filter(|e| e.is_npc()) else { return Shown::End };
        let text = if talk.female() && !entry.female_text.is_empty() { &entry.female_text } else { &entry.text };
        if text.trim_start().starts_with("G:") || text.trim_start().starts_with("g:") {
            talk.unbuilt.push("line G:".into());
            return Shown::End;
        }
        let idle = text.trim() == "I:";
        if !idle {
            let text = with_names(text, self.pc_name, &self.npc.name);
            self.float(self.npc_net, text, FloatColour::White, voice_path(self.dialog, entry.voice(), talk.female()));
        }
        if let Some(closing) = talk.run_result(&entry.result) {
            if let Some(last) = file.lines.get(&closing) {
                let text = if talk.female() && !last.female_text.is_empty() { &last.female_text } else { &last.text };
                let text = with_names(text, self.pc_name, &self.npc.name);
                self.float(self.npc_net, text, FloatColour::White, voice_path(self.dialog, last.voice(), talk.female()));
            }
            return Shown::End;
        }
        let mut shown = Vec::new();
        for (n, option) in options(file, line, talk) {
            match self.option_text(&option.text, talk) {
                Some(text) => shown.push((n, text)),
                None => talk.unbuilt.push(format!("option {}", option.text.trim().chars().take(2).collect::<String>())),
            }
        }
        if shown.is_empty() {
            return Shown::End;
        }
        Shown::Line(line, shown)
    }

    /// The text an option shows (0x416c10): as written with names, or a generic line.
    fn option_text(&mut self, text: &str, talk: &Talk) -> Option<String> {
        if !is_prefixed(text) {
            return Some(with_names(text, self.pc_name, &self.npc.name));
        }
        let range = generic_range(text)?;
        let dumb = talk.intelligence() <= 4;
        let file = match (dumb, self.npc.male) {
            (false, true) => "mes/gd_pc2m.mes",
            (false, false) => "mes/gd_pc2f.mes",
            (true, true) => "mes/gd_dumb_pc2m.mes",
            (true, false) => "mes/gd_dumb_pc2f.mes",
        };
        let lines: Vec<u32> = {
            let mes = self.generic.file(self.vfs, file)?;
            let mut keys: Vec<u32> = mes.messages.keys().copied().filter(|k| range.contains(k)).collect();
            keys.sort_unstable();
            keys
        };
        if lines.is_empty() {
            return None;
        }
        let pick = lines[self.rng.range(0, lines.len() as i32 - 1) as usize];
        let line = self.generic.file(self.vfs, file)?.get(pick)?.to_string();
        Some(with_names(&line, self.pc_name, &self.npc.name))
    }

    /// The PC chose option `line`: its result, then its response (0x413130).
    fn chosen(&mut self, file: &DialogFile, line: i32, talk: &mut Talk, depth: usize) -> Shown {
        let Some(entry) = file.lines.get(&line).filter(|e| !e.is_npc()) else { return Shown::End };
        if let Some(closing) = talk.run_result(&entry.result) {
            return self.show_closing(file, closing, talk);
        }
        match entry.response() {
            0 => Shown::End,
            n if n > 0 => self.show(file, n, talk),
            n => match file.lines.get(&-n) {
                Some(target) if target.is_npc() => self.show(file, -n, talk),
                Some(_) if depth < MAX_JUMPS => self.chosen(file, -n, talk, depth + 1),
                _ => Shown::End,
            },
        }
    }

    fn show_closing(&mut self, file: &DialogFile, line: i32, talk: &mut Talk) -> Shown {
        if let Some(last) = file.lines.get(&line) {
            let text = if talk.female() && !last.female_text.is_empty() { &last.female_text } else { &last.text };
            let text = with_names(text, self.pc_name, &self.npc.name);
            self.float(self.npc_net, text, FloatColour::White, voice_path(self.dialog, last.voice(), talk.female()));
        }
        Shown::End
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static NO_REPUTATIONS: std::sync::LazyLock<crate::ReputationEffects> = std::sync::LazyLock::new(Default::default);

    fn talk_with<'a>(rules: &'a RulesData, sheet: &'a CharacterSheet, inventory: &'a mut Inventory, pc: &'a mut PcScriptState, globals: &'a mut ScriptGlobals, npc: &'a mut Attachment) -> Talk<'a> {
        let npc_reaction = crate::NpcReaction { base: 50, race: 0, origin: 0, faction: 0, only_reputation: false };
        Talk { rules, sheet, inventory, pc, globals, npc, met: false, npc_key: "npc".into(), npc_reaction, reputations: &NO_REPUTATIONS, gold_changed: false, unbuilt: Vec::new() }
    }

    #[test]
    fn options_follow_their_line_and_pass_gender_intelligence_and_tests() {
        let file = DialogFile::parse(
            b"{1}{Hello.}{Hello, lady.}{}{1}{}{}\n\
              {2}{For men.}{1}{5}{}{10}{}\n\
              {3}{For women.}{0}{5}{}{10}{}\n\
              {4}{Clever.}{}{9}{}{10}{}\n\
              {5}{Me dumb.}{}{-4}{}{10}{}\n\
              {6}{Flag 7.}{}{5}{gf 7 1}{10}{}\n\
              {7}{Gold.}{}{1}{$$ 50}{10}{}\n\
              {8}{Next NPC line.}{}{}{}{}{}\n\
              {9}{Not shown.}{}{5}{}{10}{}",
        );
        let rules = RulesData::default();
        let sheet = CharacterSheet { gender: arcanum_rules::MALE, ..Default::default() };
        let (mut inventory, mut pc, mut globals, mut npc) = Default::default();
        let mut talk = talk_with(&rules, &sheet, &mut inventory, &mut pc, &mut globals, &mut npc);
        let iq = talk.intelligence();
        assert!((5..9).contains(&iq), "{iq}");
        let shown: Vec<i32> = options(&file, 1, &mut talk).into_iter().map(|(n, _)| n).collect();
        assert_eq!(shown, [2], "male, not clever, not dumb, no flag, no gold");
        talk.globals.set_flag(7, true);
        talk.change_gold(60);
        let shown: Vec<i32> = options(&file, 1, &mut talk).into_iter().map(|(n, _)| n).collect();
        assert_eq!(shown, [2, 6, 7]);
    }

    #[test]
    fn results_change_flags_gold_and_quests_and_skip_what_is_not_built() {
        let rules = RulesData::default();
        let sheet = CharacterSheet::default();
        let (mut inventory, mut pc, mut globals, mut npc): (Inventory, PcScriptState, ScriptGlobals, Attachment) = Default::default();
        let mut talk = talk_with(&rules, &sheet, &mut inventory, &mut pc, &mut globals, &mut npc);
        talk.change_gold(100);
        assert_eq!(talk.run_result("$$ -30, re +5, lf 2 1, lc 1 4, qu 12 2, gf 40 1, ru 7"), None);
        assert_eq!(talk.gold(), 70);
        assert!(talk.unbuilt.is_empty(), "{:?}", talk.unbuilt);
        assert_eq!(talk.npc.flags, 1 << 2);
        assert_eq!(talk.npc.counters[1], 4);
        assert!(talk.passes_test("qu 12 2, qa 12 1, qb 12 2, lf 2 1, lc 1 4, gf 40 1, ru 7"));
        assert!(!talk.passes_test("ru -7"));
        assert_eq!(talk.run_result("gv 3 9, fl 120, gv 4 1"), Some(120), "fl ends the result");
        assert_eq!((talk.globals.var(3), talk.globals.var(4)), (9, 0));
        assert!(!talk.passes_test("tr 1 1"), "not built: fails");
        assert_eq!(talk.unbuilt.last().map(String::as_str), Some("test tr"));
    }

    #[test]
    fn reaction_starts_at_the_base_plus_looks_and_changes_are_stored() {
        let rules = RulesData::default();
        let sheet = CharacterSheet::default();
        let (mut inventory, mut pc, mut globals, mut npc): (Inventory, PcScriptState, ScriptGlobals, Attachment) = Default::default();
        let mut talk = talk_with(&rules, &sheet, &mut inventory, &mut pc, &mut globals, &mut npc);
        let looks = rules.derived(&sheet).reaction;
        assert_eq!(talk.reaction(), 50 + looks, "human to human: no racial term");
        assert!(!talk.passes_test("me 1"), "not met");
        talk.run_result("re +5");
        assert_eq!(talk.reaction(), 55 + looks);
        assert!(talk.passes_test("me 1"), "a stored reaction: met");
        talk.run_result("re 61");
        assert_eq!(talk.reaction(), 61, "set");
        assert!(talk.passes_test("re 61") && !talk.passes_test("re 62") && talk.passes_test("re -61"));
        talk.run_result("re >70");
        assert_eq!(talk.reaction(), 70);
        talk.run_result("re >60, re <65");
        assert_eq!(talk.reaction(), 65);
        assert_eq!(talk.pc.reactions["npc"], 65 - looks, "the unmodified part is stored");
    }

    #[test]
    fn names_go_into_the_text() {
        assert_eq!(with_names("Welcome, @PCname@. I am @npcname@ @ home.", "Virgil", "Kietzel"), "Welcome, Virgil. I am Kietzel @ home.");
        assert_eq!(voice_path(1990, 3, true).as_deref(), Some("sound/speech/01990/v3_f.mp3"));
        assert_eq!(voice_path(1990, 0, true), None);
        assert!(is_prefixed("E:") && is_prefixed("T:5,6") && !is_prefixed("Every day."));
        assert_eq!(generic_range("E:"), Some(400..500));
        assert_eq!(generic_range("B:"), None);
    }
}
