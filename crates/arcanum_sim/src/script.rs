//! The script engine's runner (Arcanum.exe: hook 0x441980, runner 0x4449b0, conditions
//! 0x4451b0, actions 0x447220, operands 0x44ba70 / 0x44bb55, object sets 0x44b4e0). See
//! `plans/scripts.md`.
//!
//! Scripts run on the host only. [`run_hook`] runs the script attached to an object for an
//! event and says whether the event's default behaviour still happens. Its world comes through
//! [`ScriptContext`], so the runner knows nothing of maps or players: global state is
//! [`ScriptGlobals`] (saved in the [`WorldSave`](crate::WorldSave)), a PC's variables and flags
//! are [`PcScriptState`] (saved with the PC), and an attachment's flags and counters are
//! [`Attachment`] (saved with its object).
//!
//! - **Lines:** from the start line, at most [`MAX_STEPS`]. A line's condition picks its then or
//!   else action; the action's result is the next line (-1: the following one; past the last,
//!   the script ends as if by action 1), a jump, or the end: action 1 skips the default,
//!   action 2 runs it. Too many steps end it, skipping the default.
//! - **Run once** (script flag 4): ending by action 1 or 2 detaches the script. Actions 5 and 6
//!   replace the attachment's script after the run.
//! - **Built:**
//!   - conditions: 0 always, 1 daytime, 3 local flag, 4 equal, 5 at most, 0x2b global flag;
//!     0x12 (is the set's leader the other object: nobody follows anyone yet); and the flag
//!     tests on a set's objects (0xc-0x10 `spell_flags`, 0x1b/0x1e `flags`, 0x25/0x26
//!     `critter_flags`, 0x27/0x34 `npc_flags`), all members or any by the set's kind;
//!   - actions: 1-3, 4 (start a conversation, [`crate::dialogue`]), 5, 6, 8, 9, 0xa-0xf (flags, assignment, arithmetic, object variables),
//!     0x12-0x14 (loops over a set), 0x18 (a line of the script's dialogue file in the
//!     triggerer's message box), 0x25 random, 0x2a/0x2b story state, 0x60-0x62 and 0x69 (the
//!     day, hour, minute, hours since the start), 0x64/0x65 global flags;
//!   - object sets: the attachee (0), the triggerer (1), the loop's object (0xa), a local object
//!     variable (0xb), the other object (0xc).
//! - **Not built yet** (builder's call): anything else stops the script and runs the default,
//!   recorded in [`ScriptRun::unsupported`], rather than guessing at a half-run.
//!
//! Traps (scripts 30000-30007) are not built: they run the default and are reported.
//!
//! Not copied: the theft checks before a use (0x4bbfe0, 0x4bbe40); the NPC dialogue special
//! cases (event 9); the client gate (clients never run scripts here).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use arcanum_formats::script::{Block, Line, Operand, OperandKind, Script, ScriptIndex};
use serde::{Deserialize, Serialize};

/// Most lines one run steps through (0x4449b0's 1000).
pub const MAX_STEPS: usize = 1000;
/// Global variables (0x4446e0).
pub const GLOBAL_VARS: u32 = 2000;
/// Global flags: 100 words of 32 (0x4446e0).
pub const GLOBAL_FLAGS: u32 = 3200;
/// Variables local to one run.
pub const LOCAL_VARS: usize = 10;
/// Calls of scripts from scripts, deepest.
pub const MAX_DEPTH: usize = 8;

/// The state every script shares: global variables and flags and the story state (saved).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptGlobals {
    /// The variables that are not 0.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<u32, i32>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub flags: BTreeSet<u32>,
    /// Only rises (action 0x2b, 0x4450a0).
    #[serde(default)]
    pub story_state: i32,
    /// Rumours quelled (dialogue `rq`, 0x4c5690).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub quelled: BTreeSet<u32>,
}

impl ScriptGlobals {
    pub fn var(&self, index: i32) -> i32 {
        u32::try_from(index).ok().and_then(|i| self.vars.get(&i)).copied().unwrap_or(0)
    }

    pub fn set_var(&mut self, index: i32, value: i32) {
        let Some(i) = u32::try_from(index).ok().filter(|&i| i < GLOBAL_VARS) else { return };
        if value == 0 {
            self.vars.remove(&i);
        } else {
            self.vars.insert(i, value);
        }
    }

    pub fn flag(&self, index: i32) -> bool {
        u32::try_from(index).is_ok_and(|i| self.flags.contains(&i))
    }

    pub fn set_flag(&mut self, index: i32, on: bool) {
        let Some(i) = u32::try_from(index).ok().filter(|&i| i < GLOBAL_FLAGS) else { return };
        if on {
            self.flags.insert(i);
        } else {
            self.flags.remove(&i);
        }
    }
}

/// A PC's script variables and flags (fields 0x111 and 0x110), and what dialogue keeps for it:
/// quest states, rumours known, reputations, the NPCs met. Saved with the PC.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PcScriptState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<u32, i32>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub flags: BTreeSet<u32>,
    /// Quest states that are not 0 (0x4c4cb0 / 0x4c4d20).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub quests: BTreeMap<u32, i32>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub rumours: BTreeSet<u32>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub reputations: BTreeSet<u32>,
    /// The NPCs this PC has talked to ([`crate::dialogue`]).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub met: BTreeSet<String>,
    /// NPCs' stored reaction toward this PC, by NPC ([`crate::reaction`]; the original keeps
    /// it on the NPC, `npc_reaction_pc_idx` / `npc_reaction_level_idx`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reactions: BTreeMap<String, i32>,
}

impl PcScriptState {
    /// Quest `n`'s state (0 when never set).
    pub fn quest(&self, n: i32) -> i32 {
        u32::try_from(n).ok().and_then(|n| self.quests.get(&n)).copied().unwrap_or(0)
    }
}

/// A script attached to an object for one event (an entry of field 0x20): the script's own
/// flags and four counters, and its number (0: none).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub flags: u32,
    pub counters: [u8; 4],
    pub number: u32,
}

impl Attachment {
    /// From the object file's `{flags, counters, number}`.
    pub fn new(flags: u32, counters: u32, number: u32) -> Self {
        Self { flags, counters: counters.to_le_bytes(), number }
    }
}

/// What the event's caller does after the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookResult {
    /// The script handled it (the hook's 0).
    SkipDefault,
    /// The default behaviour happens (the hook's 1): no script, or the script said so.
    RunDefault,
}

/// What the runner could not do: the line and the condition or action type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsupported {
    Condition { line: usize, kind: u32 },
    Action { line: usize, kind: u32 },
    ObjectSet { line: usize, kind: i8 },
    /// A trap script ([`TRAP_SCRIPTS`]), sprung before any line runs (0x4bcb70).
    Trap { number: u32 },
}

/// The trap scripts (30000-30007): 0x4bcb70 springs the trap instead of running lines, and the
/// use does not happen.
pub const TRAP_SCRIPTS: std::ops::RangeInclusive<u32> = 30000..=30007;

/// How a run went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRun {
    pub result: HookResult,
    pub unsupported: Vec<Unsupported>,
}

/// Who a script runs for: its attachee, triggerer and an optional third object, the event
/// number (attach point) and the line to start at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invocation<O> {
    pub attachee: O,
    pub triggerer: O,
    pub other: Option<O>,
    pub event: u32,
    pub start_line: usize,
}

/// The world a script reads and changes, given by the host.
pub trait ScriptContext {
    /// An object of the world.
    type Object: Copy + Eq + std::fmt::Debug;

    /// Script `number`, if it exists.
    fn script(&mut self, number: u32) -> Option<Arc<Script>>;
    fn globals(&mut self) -> &mut ScriptGlobals;
    /// `object`'s variables and flags, if it is a PC.
    fn pc_state(&mut self, object: Self::Object) -> Option<&mut PcScriptState>;
    /// A number from `low` to `high`, both included.
    fn random(&mut self, low: i32, high: i32) -> i32;
    /// The game time.
    fn now(&self) -> crate::GameTime;
    /// Line `line` of script `script`'s dialogue file, as `reader` sees it (by its gender).
    fn dialog_text(&mut self, script: u32, line: i32, reader: Self::Object) -> Option<String>;
    /// Shows `text` in `to`'s message box (a PC's player; others: nobody).
    fn show_message(&mut self, to: Self::Object, text: String);
    /// Starts a conversation between `attachee` and `triggerer` in dialogue file `dialog` at
    /// line `line` (action 4, 0x567460).
    fn start_dialog(&mut self, _attachee: Self::Object, _triggerer: Self::Object, _dialog: u32, _line: i32) {}
    /// Field `field` of `object`'s data (its own or its prototype's); `None` for PCs and objects
    /// without it.
    fn object_int(&mut self, _object: Self::Object, _field: &str) -> Option<i32> {
        None
    }
    /// Whether `object` is an NPC.
    fn is_npc(&mut self, _object: Self::Object) -> bool {
        false
    }
    /// Asks for `object` to go to tile `to` of map `map` (a `Rules/MapList.mes` id; action 0x2c,
    /// 0x4d3380). The move happens after the run, if at all.
    fn teleport(&mut self, _object: Self::Object, _map: i32, _to: arcanum_formats::map::TileLoc) {}
    /// `object`'s leader, if it follows someone (0x45de00); nobody follows anyone yet.
    fn leader(&mut self, _object: Self::Object) -> Option<Self::Object> {
        None
    }
}

/// Object set kinds where one member passing is enough (0x4451b0); the others need all.
pub const ANY_SET_KINDS: [i8; 9] = [3, 5, 7, 9, 0xe, 0x10, 0x12, 0x14, 0x17];

/// The flag tests on a set's objects (0x4451b0): condition type, field, bits, NPCs only.
const FLAG_CONDITIONS: [(u32, &str, i32, bool); 11] = [
    (0xc, "spell_flags", 0x20000, false),
    (0xd, "spell_flags", 0x80000, false),
    (0xe, "spell_flags", 0x3c, false),
    (0xf, "spell_flags", 0x1, false),
    (0x10, "spell_flags", 0x40000, false),
    (0x1b, "flags", 0x2, false),
    (0x1e, "flags", 0x400000, false),
    (0x25, "critter_flags", 0x8, true),
    (0x26, "critter_flags", 0x4, true),
    (0x27, "npc_flags", 0x20, true),
    (0x34, "npc_flags", 0x8, true),
];

/// Script numbers to parsed scripts: the index of `scr/` and the scripts and dialogue files
/// read so far.
#[derive(Debug, Default)]
pub struct ScriptLibrary {
    pub index: ScriptIndex,
    loaded: std::collections::HashMap<u32, Option<Arc<Script>>>,
    dialogs: std::collections::HashMap<u32, Option<Arc<arcanum_formats::dialog::DialogFile>>>,
}

impl ScriptLibrary {
    /// Indexes the scripts in the game data.
    pub fn new(vfs: &arcanum_vfs::Vfs) -> Self {
        let names = vfs.list_dir(arcanum_formats::script::DIR).unwrap_or_default();
        let index = ScriptIndex::new(names.iter().map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p)));
        Self { index, loaded: Default::default(), dialogs: Default::default() }
    }

    /// Script `number`, read on first use; `None` if it is not there or does not parse.
    pub fn get(&mut self, vfs: &arcanum_vfs::Vfs, number: u32) -> Option<Arc<Script>> {
        let index = &self.index;
        self.loaded.entry(number).or_insert_with(|| index.path(number).and_then(|p| vfs.read(&p).ok()).and_then(|d| Script::parse(&d).ok()).map(Arc::new)).clone()
    }

    /// Script `number`'s dialogue file (`dlg/` beside `scr/`), read on first use.
    pub fn dialog(&mut self, vfs: &arcanum_vfs::Vfs, number: u32) -> Option<Arc<arcanum_formats::dialog::DialogFile>> {
        let index = &self.index;
        self.dialogs
            .entry(number)
            .or_insert_with(|| index.path(number).and_then(|p| arcanum_formats::dialog::dialog_path(&p)).and_then(|p| vfs.read(&p).ok()).map(|d| Arc::new(arcanum_formats::dialog::DialogFile::parse(&d))))
            .clone()
    }

    /// Puts a script in by hand (tests).
    pub fn insert(&mut self, number: u32, script: Script) {
        self.loaded.insert(number, Some(Arc::new(script)));
    }

    /// Puts a dialogue file in by hand (tests).
    pub fn insert_dialog(&mut self, number: u32, dialog: arcanum_formats::dialog::DialogFile) {
        self.dialogs.insert(number, Some(Arc::new(dialog)));
    }
}

/// Runs the script `attachment` holds for `invocation` (0x441980 / 0x4449b0). No script: the
/// default runs.
pub fn run_hook<C: ScriptContext>(ctx: &mut C, attachment: &mut Attachment, invocation: Invocation<C::Object>) -> ScriptRun {
    let mut unsupported = Vec::new();
    let result = run(ctx, attachment, invocation, 0, &mut unsupported);
    ScriptRun { result, unsupported }
}

/// How a line ended.
enum Step {
    Next,
    Jump(usize),
    SkipDefault,
    RunDefault,
    Stop,
}

/// One run's state (0x4449b0's context).
struct Run<O> {
    invocation: Invocation<O>,
    locals: [i32; LOCAL_VARS],
    objects: [Option<O>; LOCAL_VARS],
    /// The loop over a set: its objects, where it is, and the line after its start.
    looping: Option<(Vec<O>, usize, usize)>,
    /// A new script number for the attachment after the run (actions 5 and 6).
    replace: Option<u32>,
}

fn run<C: ScriptContext>(ctx: &mut C, attachment: &mut Attachment, invocation: Invocation<C::Object>, depth: usize, unsupported: &mut Vec<Unsupported>) -> HookResult {
    if attachment.number == 0 || depth > MAX_DEPTH {
        return HookResult::RunDefault;
    }
    if TRAP_SCRIPTS.contains(&attachment.number) {
        unsupported.push(Unsupported::Trap { number: attachment.number });
        return HookResult::RunDefault;
    }
    let Some(script) = ctx.script(attachment.number) else { return HookResult::RunDefault };
    let mut run = Run { invocation, locals: [0; LOCAL_VARS], objects: [None; LOCAL_VARS], looping: None, replace: None };
    let mut line = invocation.start_line;
    let mut result = HookResult::SkipDefault;
    let mut ended_by_action = false;
    for _ in 0..MAX_STEPS {
        let Some(current) = script.lines.get(line) else { break };
        match step(ctx, attachment, &mut run, &script, line, current, unsupported) {
            Step::Next if line + 1 < script.lines.len() => line += 1,
            Step::Next | Step::SkipDefault => {
                ended_by_action = true;
                break;
            }
            Step::Jump(to) => line = to,
            Step::RunDefault => {
                (result, ended_by_action) = (HookResult::RunDefault, true);
                break;
            }
            Step::Stop => {
                result = HookResult::RunDefault;
                break;
            }
        }
    }
    if ended_by_action && script.runs_once() {
        attachment.number = 0;
    }
    if let Some(number) = run.replace {
        attachment.number = number;
    }
    result
}

fn step<C: ScriptContext>(ctx: &mut C, attachment: &mut Attachment, run: &mut Run<C::Object>, script: &Script, line: usize, current: &Line, unsupported: &mut Vec<Unsupported>) -> Step {
    let holds = match condition(ctx, attachment, run, &current.condition, line, unsupported) {
        Some(holds) => holds,
        None => {
            unsupported.push(Unsupported::Condition { line, kind: current.condition.kind });
            return Step::Stop;
        }
    };
    let block = if holds { &current.then } else { &current.otherwise };
    match action(ctx, attachment, run, script, line, block, unsupported) {
        Some(step) => step,
        None => {
            unsupported.push(Unsupported::Action { line, kind: block.kind });
            Step::Stop
        }
    }
}

/// A condition's truth (0x4451b0); `None` when it is not built.
fn condition<C: ScriptContext>(ctx: &mut C, attachment: &mut Attachment, run: &mut Run<C::Object>, block: &Block, line: usize, unsupported: &mut Vec<Unsupported>) -> Option<bool> {
    // Tests on the members of the set in operand 0: all must pass, or any for the "any" kinds.
    let over_set = |ctx: &mut C, run: &Run<C::Object>, unsupported: &mut Vec<Unsupported>, test: &mut dyn FnMut(&mut C, C::Object) -> bool| -> Option<bool> {
        let set = block.operands[0];
        let members = objects(run, set, line, unsupported)?;
        let passed = members.iter().filter(|&&o| test(ctx, o)).count();
        Some(if ANY_SET_KINDS.contains(&set.tag) { passed > 0 } else { passed == members.len() && passed > 0 })
    };
    if let Some(&(_, field, bits, npc_only)) = FLAG_CONDITIONS.iter().find(|c| c.0 == block.kind) {
        return over_set(ctx, run, unsupported, &mut |ctx, o| (!npc_only || ctx.is_npc(o)) && ctx.object_int(o, field).is_some_and(|v| v & bits != 0));
    }
    if block.kind == 0x12 {
        // The set's leader is the object in operand 1 (0x45de00).
        let other = objects(run, block.operands[1], line, unsupported)?.first().copied();
        return over_set(ctx, run, unsupported, &mut |ctx, o| other.is_some() && ctx.leader(o) == other);
    }
    let mut value = |i: usize| get(ctx, attachment, run, block.operands[i]);
    Some(match block.kind {
        0 => true,
        1 => ctx.now().is_day(),
        3 => {
            let bit = value(0);
            attachment.flags & (1 << (bit & 31)) != 0
        }
        4 => value(0) == value(1),
        5 => value(0) <= value(1),
        0x2b => {
            let flag = value(0);
            ctx.globals().flag(flag)
        }
        _ => return None,
    })
}

/// An action (0x447220); `None` when it is not built.
fn action<C: ScriptContext>(ctx: &mut C, attachment: &mut Attachment, run: &mut Run<C::Object>, script: &Script, line: usize, block: &Block, unsupported: &mut Vec<Unsupported>) -> Option<Step> {
    let ops = block.operands;
    let value = |ctx: &mut C, attachment: &mut Attachment, run: &mut Run<C::Object>, i: usize| get(ctx, attachment, run, ops[i]);
    let arithmetic = |a: i32, b: i32, kind: u32| match kind {
        0xb => Some(a.wrapping_add(b)),
        0xc => Some(a.wrapping_sub(b)),
        0xd => Some(a.wrapping_mul(b)),
        _ => (b != 0).then(|| a.wrapping_div(b)),
    };
    Some(match block.kind {
        0 => Step::Next,
        1 => Step::SkipDefault,
        2 => Step::RunDefault,
        3 => Step::Jump(value(ctx, attachment, run, 0).max(0) as usize),
        4 => {
            // Dialogue in the running script's own file, at line operand 0; the script ends,
            // skipping the default (0x447282).
            let line = value(ctx, attachment, run, 0);
            ctx.start_dialog(run.invocation.attachee, run.invocation.triggerer, attachment.number, line);
            Step::SkipDefault
        }
        5 => {
            run.replace = Some(0);
            Step::Next
        }
        6 => {
            run.replace = Some(value(ctx, attachment, run, 0).max(0) as u32);
            Step::Next
        }
        // Teleport (0x4481a1): objects, map, x, y; queued, the run goes on. 0x66 (0x449a85) is
        // the same with a fade, a sound and a delay first (operands 0-2, not built); 0x67 is a
        // fade alone (not built: nothing happens). The original skips 0x66 and 0x67 in
        // multiplayer ([0x62ac6c] & 1); here 0x66 teleports as 0x2c does.
        0x2c | 0x66 => {
            let first = if block.kind == 0x2c { 0 } else { 3 };
            let who = objects(run, ops[first], line, unsupported)?;
            let map = value(ctx, attachment, run, first + 1);
            let (x, y) = (value(ctx, attachment, run, first + 2), value(ctx, attachment, run, first + 3));
            let to = arcanum_formats::map::TileLoc { x: x.max(0) as u32, y: y.max(0) as u32 };
            for object in who {
                ctx.teleport(object, map, to);
            }
            Step::Next
        }
        0x67 => Step::Next,
        8 | 9 => {
            let bit = 1 << (value(ctx, attachment, run, 0) & 31);
            attachment.flags = if block.kind == 8 { attachment.flags | bit } else { attachment.flags & !bit };
            Step::Next
        }
        0xa => {
            let v = value(ctx, attachment, run, 1);
            set(ctx, attachment, run, ops[0], v);
            Step::Next
        }
        0xb..=0xe => {
            let (a, b) = (value(ctx, attachment, run, 1), value(ctx, attachment, run, 2));
            if let Some(v) = arithmetic(a, b, block.kind) {
                set(ctx, attachment, run, ops[0], v);
            }
            Step::Next
        }
        0xf => {
            let object = objects(run, ops[1], line, unsupported)?.first().copied();
            if ops[0].tag == 0xb
                && let Some(slot) = usize::try_from(ops[0].value).ok().and_then(|i| run.objects.get_mut(i))
            {
                *slot = object;
            }
            Step::Next
        }
        0x12 => {
            if run.looping.is_some() {
                return Some(Step::Next);
            }
            let set = objects(run, ops[0], line, unsupported)?;
            if set.is_empty() {
                return Some(loop_exit(script, line + 1));
            }
            run.looping = Some((set, 0, line + 1));
            Step::Next
        }
        0x13 => {
            let Some((set, at, start)) = run.looping.as_mut() else { return Some(Step::Next) };
            *at += 1;
            if *at < set.len() {
                Step::Jump(*start)
            } else {
                run.looping = None;
                Step::Next
            }
        }
        0x14 => {
            let Some((_, _, start)) = run.looping.take() else { return Some(Step::Next) };
            loop_exit(script, start)
        }
        0x18 => {
            // A line of the script's dialogue file in the triggerer's message box (0x44b390).
            let number = value(ctx, attachment, run, 0);
            let triggerer = run.invocation.triggerer;
            if let Some(text) = ctx.dialog_text(attachment.number, number, triggerer) {
                ctx.show_message(triggerer, text);
            }
            Step::Next
        }
        0x60 | 0x61 | 0x62 | 0x69 => {
            // The day (0x45a8d0: days + 1), the hour (0x45a870), the minute (0x45a8b0), the
            // hours since the start (0x45a890: days * 24 + hour).
            let now = ctx.now();
            let v = match block.kind {
                0x60 => now.days as i32 + 1,
                0x61 => now.hour() as i32,
                0x62 => now.minute() as i32,
                _ => (now.days * 24 + now.hour()) as i32,
            };
            set(ctx, attachment, run, ops[0], v);
            Step::Next
        }
        0x25 => {
            let (a, b) = (value(ctx, attachment, run, 0), value(ctx, attachment, run, 1));
            let v = ctx.random(a.min(b), a.max(b));
            set(ctx, attachment, run, ops[2], v);
            Step::Next
        }
        0x2a => {
            let v = ctx.globals().story_state;
            set(ctx, attachment, run, ops[0], v);
            Step::Next
        }
        0x2b => {
            let v = value(ctx, attachment, run, 0);
            let globals = ctx.globals();
            globals.story_state = globals.story_state.max(v);
            Step::Next
        }
        0x64 | 0x65 => {
            let flag = value(ctx, attachment, run, 0);
            ctx.globals().set_flag(flag, block.kind == 0x64);
            Step::Next
        }
        _ => return None,
    })
}

/// Where a loop's break goes (0x44bc60): past the first line from `from` that always ends the
/// loop (condition 0, then action 0x13).
fn loop_exit(script: &Script, from: usize) -> Step {
    let end = (from..script.lines.len()).find(|&i| script.lines[i].condition.kind == 0 && script.lines[i].then.kind == 0x13);
    match end {
        Some(i) => Step::Jump(i + 1),
        None => Step::SkipDefault,
    }
}

/// The objects an object operand names (0x44b4e0): its tag is the set kind.
fn objects<O: Copy>(run: &Run<O>, operand: Operand, line: usize, unsupported: &mut Vec<Unsupported>) -> Option<Vec<O>> {
    let inv = &run.invocation;
    Some(match operand.tag {
        // Set 0 is the hook's first object, the triggerer; 1 its second, the attachee whose
        // script runs (0x44b4e0 reads +0x8 and +0x10 of the run's objects, which 0x441980
        // fills with its first and second arguments; checked 2026-11-24).
        0 => vec![inv.triggerer],
        1 => vec![inv.attachee],
        0xa => run.looping.as_ref().and_then(|(set, at, _)| set.get(*at)).copied().into_iter().collect(),
        0xb => usize::try_from(operand.value).ok().and_then(|i| run.objects.get(i).copied().flatten()).into_iter().collect(),
        0xc => inv.other.into_iter().collect(),
        kind => {
            unsupported.push(Unsupported::ObjectSet { line, kind });
            return None;
        }
    })
}

/// The object a PC variable or flag operand names: the first of the set in its high 16 bits.
fn pc_object<O: Copy>(run: &Run<O>, value: i32) -> Option<O> {
    let set = Operand { tag: (value >> 16) as i8, value: 0 };
    objects(run, set, 0, &mut Vec::new())?.first().copied()
}

/// A scalar operand's value (0x44ba70).
fn get<C: ScriptContext>(ctx: &mut C, attachment: &Attachment, run: &Run<C::Object>, operand: Operand) -> i32 {
    let v = operand.value;
    match operand.kind() {
        OperandKind::Counter => usize::try_from(v).ok().and_then(|i| attachment.counters.get(i)).map_or(0, |&c| i32::from(c)),
        OperandKind::GlobalVar => ctx.globals().var(v),
        OperandKind::Local => usize::try_from(v).ok().and_then(|i| run.locals.get(i)).copied().unwrap_or(0),
        OperandKind::GlobalFlag => i32::from(ctx.globals().flag(v)),
        OperandKind::PcVar | OperandKind::PcFlag => {
            let index = (v & 0xffff) as u32;
            let Some(state) = pc_object(run, v).and_then(|o| ctx.pc_state(o)) else { return 0 };
            if operand.kind() == OperandKind::PcVar { state.vars.get(&index).copied().unwrap_or(0) } else { i32::from(state.flags.contains(&index)) }
        }
        OperandKind::Number | OperandKind::Other(_) => v,
    }
}

/// Writes a scalar operand (0x44bb55); numbers are not written.
fn set<C: ScriptContext>(ctx: &mut C, attachment: &mut Attachment, run: &mut Run<C::Object>, operand: Operand, value: i32) {
    let v = operand.value;
    match operand.kind() {
        OperandKind::Counter => {
            if let Some(c) = usize::try_from(v).ok().and_then(|i| attachment.counters.get_mut(i)) {
                *c = value as u8;
            }
        }
        OperandKind::GlobalVar => ctx.globals().set_var(v, value),
        OperandKind::Local => {
            if let Some(l) = usize::try_from(v).ok().and_then(|i| run.locals.get_mut(i)) {
                *l = value;
            }
        }
        OperandKind::GlobalFlag => ctx.globals().set_flag(v, value != 0),
        OperandKind::PcVar | OperandKind::PcFlag => {
            let index = (v & 0xffff) as u32;
            let Some(state) = pc_object(run, v).and_then(|o| ctx.pc_state(o)) else { return };
            match (operand.kind(), value) {
                (OperandKind::PcVar, 0) => {
                    state.vars.remove(&index);
                }
                (OperandKind::PcVar, value) => {
                    state.vars.insert(index, value);
                }
                (_, 0) => {
                    state.flags.remove(&index);
                }
                _ => {
                    state.flags.insert(index);
                }
            }
        }
        OperandKind::Number | OperandKind::Other(_) => {}
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use arcanum_formats::script::{Block, Line, OPERANDS};

    /// A world for tests: objects are numbers; 1 and 2 are PCs.
    #[derive(Default)]
    pub struct TestWorld {
        pub scripts: std::collections::HashMap<u32, Arc<Script>>,
        pub globals: ScriptGlobals,
        pub pcs: BTreeMap<u32, PcScriptState>,
        pub now: crate::GameTime,
        pub messages: Vec<(u32, String)>,
    }

    impl ScriptContext for TestWorld {
        type Object = u32;
        fn script(&mut self, number: u32) -> Option<Arc<Script>> {
            self.scripts.get(&number).cloned()
        }
        fn globals(&mut self) -> &mut ScriptGlobals {
            &mut self.globals
        }
        fn pc_state(&mut self, object: u32) -> Option<&mut PcScriptState> {
            self.pcs.get_mut(&object)
        }
        fn random(&mut self, low: i32, _high: i32) -> i32 {
            low
        }
        fn now(&self) -> crate::GameTime {
            self.now
        }
        fn dialog_text(&mut self, script: u32, line: i32, _reader: u32) -> Option<String> {
            Some(format!("{script}:{line}"))
        }
        fn show_message(&mut self, to: u32, text: String) {
            self.messages.push((to, text));
        }
    }

    pub fn op(tag: i8, value: i32) -> Operand {
        Operand { tag, value }
    }

    pub fn n(value: i32) -> Operand {
        op(3, value)
    }

    pub fn block(kind: u32, operands: &[Operand]) -> Block {
        let mut ops = [Operand::default(); OPERANDS];
        ops[..operands.len()].copy_from_slice(operands);
        Block { kind, operands: ops }
    }

    /// "If `condition` then `then` else `otherwise`".
    pub fn line(condition: Block, then: Block, otherwise: Block) -> Line {
        Line { condition, then, otherwise }
    }

    /// An unconditional line.
    pub fn always(then: Block) -> Line {
        line(block(0, &[]), then, block(0, &[]))
    }

    fn world(lines: Vec<Line>, flags: u32) -> TestWorld {
        let mut w = TestWorld { pcs: BTreeMap::from([(1, PcScriptState::default()), (2, PcScriptState::default())]), ..Default::default() };
        w.scripts.insert(100, Arc::new(Script { description: String::new(), flags, lines }));
        w
    }

    fn invoke(w: &mut TestWorld, attachment: &mut Attachment) -> ScriptRun {
        run_hook(w, attachment, Invocation { attachee: 50, triggerer: 1, other: Some(2), event: 1, start_line: 0 })
    }

    #[test]
    fn no_script_runs_the_default_and_the_end_of_the_lines_skips_it() {
        let mut w = world(vec![always(block(0, &[]))], 0);
        assert_eq!(invoke(&mut w, &mut Attachment::default()).result, HookResult::RunDefault);
        assert_eq!(invoke(&mut w, &mut Attachment::new(0, 0, 999)).result, HookResult::RunDefault, "missing script");
        assert_eq!(invoke(&mut w, &mut Attachment::new(0, 0, 100)).result, HookResult::SkipDefault);
    }

    #[test]
    fn conditions_choose_then_or_else() {
        // As 01621Open_Bed_CM.scr: global flag 2273 set runs the default, else skips it.
        let lines = vec![line(block(4, &[op(4, 2273), n(1)]), block(2, &[]), block(1, &[]))];
        let mut w = world(lines, 0);
        assert_eq!(invoke(&mut w, &mut Attachment::new(0, 0, 100)).result, HookResult::SkipDefault);
        w.globals.set_flag(2273, true);
        assert_eq!(invoke(&mut w, &mut Attachment::new(0, 0, 100)).result, HookResult::RunDefault);
    }

    #[test]
    fn goto_arithmetic_and_every_kind_of_variable() {
        let lines = vec![
            always(block(0xa, &[op(2, 0), n(7)])),                 // local 0 = 7
            always(block(0xb, &[op(1, 5), op(2, 0), n(3)])),       // global var 5 = 10
            always(block(0xe, &[op(0, 2), op(1, 5), n(0)])),       // divide by 0: nothing
            always(block(0xd, &[op(0, 2), op(1, 5), n(2)])),       // counter 2 = 20
            always(block(0xa, &[op(5, 9), n(4)])),                 // the triggerer's (set 0) PC var 9 = 4
            always(block(0xa, &[op(6, (0xc << 16) | 3), n(1)])),   // the other's PC flag 3
            always(block(0xa, &[op(6, (1 << 16) | 40), n(1)])),    // the attachee (set 1) is no PC: ignored
            always(block(8, &[n(6)])),                             // local flag 6
            line(block(5, &[op(0, 2), n(19)]), block(1, &[]), block(3, &[n(10)])),
            always(block(1, &[])),
            always(block(0x64, &[n(77)])),                         // line 10
        ];
        let mut w = world(lines, 0);
        let mut attachment = Attachment::new(0, 0, 100);
        let run = invoke(&mut w, &mut attachment);
        assert_eq!(run, ScriptRun { result: HookResult::SkipDefault, unsupported: vec![] });
        assert_eq!(w.globals.var(5), 10);
        assert_eq!(attachment.counters, [0, 0, 20, 0]);
        assert_eq!(attachment.flags, 1 << 6);
        assert_eq!(w.pcs[&1].vars.get(&9), Some(&4));
        assert!(w.pcs[&2].flags.contains(&3));
        assert!(w.globals.flag(77), "20 > 19: jumped to line 10");
    }

    #[test]
    fn run_once_scripts_detach_and_scripts_can_replace_themselves() {
        let mut w = world(vec![always(block(2, &[]))], arcanum_formats::script::RUN_ONCE);
        let mut attachment = Attachment::new(0, 0, 100);
        assert_eq!(invoke(&mut w, &mut attachment).result, HookResult::RunDefault);
        assert_eq!(attachment.number, 0);
        let mut w = world(vec![always(block(6, &[n(321)])), always(block(1, &[]))], 0);
        let mut attachment = Attachment::new(0, 0, 100);
        invoke(&mut w, &mut attachment);
        assert_eq!(attachment.number, 321);
    }

    #[test]
    fn endless_scripts_stop_and_unknown_types_run_the_default() {
        let mut w = world(vec![always(block(3, &[n(0)]))], 0);
        assert_eq!(invoke(&mut w, &mut Attachment::new(0, 0, 100)).result, HookResult::SkipDefault);
        let mut w = world(vec![always(block(0x8d, &[]))], 0);
        let run = invoke(&mut w, &mut Attachment::new(0, 0, 100));
        assert_eq!(run.result, HookResult::RunDefault);
        assert_eq!(run.unsupported, vec![Unsupported::Action { line: 0, kind: 0x8d }]);
    }

    #[test]
    fn loops_visit_each_object_and_break_past_their_end() {
        // For the triggerer and the other (set 0xc is one object; loop over a local list).
        let lines = vec![
            always(block(0xf, &[op(0xb, 0), op(1, 0)])),            // local object 0 = triggerer
            always(block(0x12, &[op(0xb, 0)])),                     // for each in it
            always(block(0xb, &[op(2, 1), op(2, 1), n(1)])),        // local 1 += 1
            line(block(5, &[n(1), op(2, 1)]), block(0x14, &[]), block(0, &[])), // break at once
            always(block(0x13, &[])),                               // end
            always(block(0xa, &[op(1, 1), op(2, 1)])),              // global var 1 = local 1
            always(block(0x2b, &[n(3)])),
            always(block(0x2b, &[n(2)])),
            always(block(0x2a, &[op(1, 2)])),
        ];
        let mut w = world(lines, 0);
        invoke(&mut w, &mut Attachment::new(0, 0, 100));
        assert_eq!(w.globals.var(1), 1, "one object: once round");
        assert_eq!((w.globals.story_state, w.globals.var(2)), (3, 3), "the story state only rises");
    }

    #[test]
    fn the_inn_bed_check_compares_hours_and_shows_a_dialogue_line() {
        // As 01990Inn_Bed_Use.scr, lines 27-31: paid until global var 1043 (hours); "now <=
        // paid" sleeps, else line 2 of its dialogue and the default (refused).
        let lines = vec![
            always(block(0xa, &[op(2, 1), op(1, 1043)])),
            always(block(0x69, &[op(2, 0)])),
            line(block(5, &[op(2, 0), op(2, 1)]), block(1, &[]), block(0, &[])),
            always(block(0x18, &[n(2), n(6)])),
            always(block(2, &[])),
        ];
        let mut w = world(lines, 0);
        w.now = crate::GameTime { days: 2, ms: 5 * crate::HOUR_MS };
        assert_eq!(invoke(&mut w, &mut Attachment::new(0, 0, 100)).result, HookResult::RunDefault);
        assert_eq!(w.messages, vec![(1, "100:2".to_string())], "to the triggerer");
        w.globals.set_var(1043, 2 * 24 + 5);
        assert_eq!(invoke(&mut w, &mut Attachment::new(0, 0, 100)).result, HookResult::SkipDefault, "paid until this hour");
    }
}
