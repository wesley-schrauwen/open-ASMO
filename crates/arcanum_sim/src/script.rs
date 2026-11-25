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
    /// Entries of the 256-entry table at 0x5fc350 switched on (bit 0x80): action 0x6e flips
    /// one, condition 0x2e tests one. Its other users (0x4ba5xx) read an object field's bits
    /// 21+: probably monster generators (not traced).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub switches: BTreeSet<u8>,
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
    /// The areas this PC knows ([`crate::areas`], 0x4caf50).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub areas: BTreeSet<u32>,
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
    /// Gold `object` carries (0x464780): a PC's, an NPC's or a container's.
    fn gold(&mut self, _object: Self::Object) -> i32 {
        0
    }
    /// Gives `n` gold to `object`, or takes `-n` (0x464830): nothing when it has too little.
    fn change_gold(&mut self, _object: Self::Object, _n: i32) {}
    /// Whether `object` carries an item whose `name` is `name` (0x462480).
    fn has_item_named(&mut self, _object: Self::Object, _name: i32) -> bool {
        false
    }
    /// Moves the first item named `name` from `from` to `to` (0x4617f0); no room, it stays.
    fn transfer_item_named(&mut self, _name: i32, _from: Self::Object, _to: Self::Object) {}
    /// Makes an item of basic prototype `basic` inside `into` (0x4ed8b0 → 0x4617f0).
    fn create_item(&mut self, _basic: i32, _into: Self::Object) {}
    /// Whether `object` is dead (0x45d8d0: hit points below 1).
    fn is_dead(&mut self, _object: Self::Object) -> bool {
        false
    }
    /// `object`'s type (`obj_f_type`: 15 a PC, 16 an NPC, 2 a container, 5-14 items).
    fn object_type(&mut self, _object: Self::Object) -> i32 {
        0
    }
    /// Floats `text` over `object` (0x44b1f0 → 0x568540).
    fn float_text(&mut self, _object: Self::Object, _text: String) {}
    /// `object`'s attachment at `event` (0x407840).
    fn attachment(&mut self, _object: Self::Object, _event: u32) -> Attachment {
        Attachment::default()
    }
    /// Keeps `object`'s changed attachment at `event` (0x4f01d0).
    fn set_attachment(&mut self, _object: Self::Object, _event: u32, _attachment: Attachment) {}
    /// The objects of `kind` around `centre` (0x440fc0).
    fn near(&mut self, _centre: Self::Object, _kind: NearKind) -> Vec<Self::Object> {
        Vec::new()
    }
    /// `object`'s effective stat `stat` (0x4b0490: 0-7 the primary stats, 8-15 derived, 0x10
    /// the aptitude, 0x11 on level, experience, alignment, fate points, unspent points, magick
    /// and tech points, poison, age, gender, race); 0 for objects that are not critters.
    fn stat(&mut self, _object: Self::Object, _stat: i32) -> i32 {
        0
    }
    /// `npc`'s reaction to `pc` (0x4c0cc0); 50 when they are not an NPC and a PC.
    fn reaction(&mut self, _npc: Self::Object, _pc: Self::Object) -> i32 {
        50
    }
    /// Changes `npc`'s reaction to `pc` by `delta` (0x4c0de0).
    fn change_reaction(&mut self, _npc: Self::Object, _pc: Self::Object, _delta: i32) {}
    /// The distance between two objects, in tiles (0x4b96f0; not read: here the straight line).
    fn distance(&mut self, _a: Self::Object, _b: Self::Object) -> i32 {
        0
    }
    /// Whether `npc` has met `pc` (0x4c0c40); the same object has.
    fn has_met(&mut self, npc: Self::Object, pc: Self::Object) -> bool {
        npc == pc
    }
    /// Whether `object` knows spell `spell` (0x4b1950: its college's level at least the spell's
    /// place in it plus one).
    fn knows_spell(&mut self, _object: Self::Object, _spell: i32) -> bool {
        false
    }
    /// Runs script `script` from `line` for `triggerer` and `attachee` in `delay_ms` game
    /// milliseconds (action 0x36, time event type 4).
    fn schedule(&mut self, _script: u32, _line: i32, _triggerer: Self::Object, _attachee: Self::Object, _delay_ms: u64) {}
    /// The area `object` is in (0x4cb6a0); 0 for none.
    fn area_of(&mut self, _object: Self::Object) -> i32 {
        0
    }
    /// "The player" (set 0x15, 0x40da50): the acting player's PC.
    fn player(&mut self) -> Option<Self::Object> {
        None
    }
    /// Damages `object` by `amount` of kind `kind` (0x3c → 0x4b2210 / 0x4b4390): 0 normal, 2
    /// electrical, 3 fire (hit points), 1 poison (stat 0x18), 4 fatigue (0x45d5a0).
    fn damage(&mut self, _object: Self::Object, _amount: i32, _kind: i32) {}
    /// Casts spell `spell` on `target` with no caster (0x3d → 0x455a20 / 0x455ac0).
    fn cast_on(&mut self, _spell: i32, _target: Self::Object) {}
    /// Makes `object` a new object of prototype `prototype` in its place, keeping how hurt it is
    /// (0x28).
    fn transform(&mut self, _object: Self::Object, _prototype: i32) {}
    /// Shows text window `kind` (0 book, 6 plaque) from line `line` to `to`'s player (0x4ee290).
    fn show_text(&mut self, _kind: i32, _line: i32, _to: Self::Object) {}
    /// Sets the number in `object`'s art id (0x3b → 0x502780), frame 0, if that art exists.
    fn set_art_number(&mut self, _object: Self::Object, _number: i32) {}
    /// Skill `k`'s level, 0-15 (0x2f → 0x4c5eb0 / 0x4c65e0); 0 for objects that are not critters.
    fn skill_level(&mut self, _object: Self::Object, _k: i32) -> i32 {
        0
    }
    /// Destroys `object` (0x44 → 0x43cca0).
    fn destroy(&mut self, _object: Self::Object) {}
    /// Whether `object` wears an item whose `name` is `name` (slots 1000-1008, 0x4649c0).
    fn wears_item_named(&mut self, _object: Self::Object, _name: i32) -> bool {
        false
    }
    /// Flips `mask` in `object`'s `flags` (actions 0x38 / 0x39: 0x4efee0 clears a set bit,
    /// 0x4eff50 sets a clear one), any type.
    fn toggle_flags(&mut self, _object: Self::Object, _mask: i32) {}
    /// Gives `object` effect `id` for `cause` (0x4e9f70): critters only, appended.
    fn give_effect(&mut self, _object: Self::Object, _id: i32, _cause: i32) {}
    /// Takes the oldest hold of effect `id` from `object` (0x4ea100).
    fn remove_effect(&mut self, _object: Self::Object, _id: i32) {}
    /// `object`'s leader, if it follows someone (0x45de00); nobody follows anyone yet.
    fn leader(&mut self, _object: Self::Object) -> Option<Self::Object> {
        None
    }
}

/// What a vicinity set lists (0x44b4e0's type masks for 0x440fc0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NearKind {
    /// PCs and NPCs (sets 8/9, mask 0x18000).
    Critters,
    /// Scenery (0xf/0x10, mask 8).
    Scenery,
    /// Containers (0x11/0x12, mask 4).
    Containers,
    /// Portals (0x13/0x14, mask 2).
    Portals,
    /// Items (0x16/0x17, mask 0x7fe0).
    Items,
}

impl NearKind {
    /// The vicinity set an object set id names.
    pub fn of_set(set: i8) -> Option<Self> {
        Some(match set {
            8 | 9 => Self::Critters,
            0xf | 0x10 => Self::Scenery,
            0x11 | 0x12 => Self::Containers,
            0x13 | 0x14 => Self::Portals,
            0x16 | 0x17 => Self::Items,
            _ => return None,
        })
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
    /// How deep in called scripts (action 0x51).
    depth: usize,
    /// The vicinity sets around the attachee and the player (set 0x15), read at the start for
    /// scripts that name them.
    near: BTreeMap<NearKind, Vec<O>>,
    player: Option<O>,
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
    let names = |tags: &[i8]| script.lines.iter().flat_map(|l| [&l.condition, &l.then, &l.otherwise]).any(|b| b.operands.iter().any(|o| tags.contains(&o.tag)));
    let mut near = BTreeMap::new();
    for set in [8, 0xf, 0x11, 0x13, 0x16] {
        let kind = NearKind::of_set(set).expect("a vicinity set");
        if names(&[set, set + 1]) {
            near.insert(kind, ctx.near(invocation.attachee, kind));
        }
    }
    let player = if names(&[0x15]) { ctx.player() } else { None };
    let mut run = Run { invocation, locals: [0; LOCAL_VARS], objects: [None; LOCAL_VARS], looping: None, replace: None, depth, near, player };
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
    // Tests on each member of the set in operand 0 (0x4451b0).
    let number = |ctx: &mut C, run: &Run<C::Object>, i: usize| get(ctx, attachment, run, block.operands[i]);
    match block.kind {
        2 => {
            let n = number(ctx, run, 1);
            return over_set(ctx, run, unsupported, &mut |ctx, o| ctx.gold(o) >= n);
        }
        // The quest's state, exactly (0x4c4cb0: botched reads 6); not a PC: the global quest
        // state (not built: 0).
        6 => {
            let (quest, state) = (number(ctx, run, 1), number(ctx, run, 2));
            return over_set(ctx, run, unsupported, &mut |ctx, o| ctx.pc_state(o).map_or(0, |s| s.quest(quest)) == state);
        }
        0x11 => {
            let name = number(ctx, run, 1);
            return over_set(ctx, run, unsupported, &mut |ctx, o| ctx.has_item_named(o, name));
        }
        // Wears an item named N (0x445d77: slots 1000-1008).
        0x15 => {
            let name = number(ctx, run, 1);
            return over_set(ctx, run, unsupported, &mut |ctx, o| ctx.wears_item_named(o, name));
        }
        // A critter with `critter_flags` 0x40000 (0x4b3d50; meaning not traced).
        0x1f => return over_set(ctx, run, unsupported, &mut |ctx, o| matches!(ctx.object_type(o), 15 | 16) && ctx.object_int(o, "critter_flags").unwrap_or(0) & 0x4_0000 != 0),
        // Named, and not flagged 0x8000000 (`flags`; probably destroyed).
        0x14 => {
            let name = number(ctx, run, 1);
            return over_set(ctx, run, unsupported, &mut |ctx, o| ctx.object_int(o, "name") == Some(name) && ctx.object_int(o, "flags").unwrap_or(0) & 0x800_0000 == 0);
        }
        0x16 => return over_set(ctx, run, unsupported, &mut |ctx, o| ctx.is_dead(o)),
        // Identified (0x464370): an item with no complexity, or flagged identified.
        0x2f => {
            return over_set(ctx, run, unsupported, &mut |ctx, o| {
                let kind = ctx.object_type(o);
                (5..=14).contains(&kind) && (ctx.object_int(o, "item_magic_tech_complexity").unwrap_or(0) < 1 || ctx.object_int(o, "item_flags").unwrap_or(0) & 1 != 0)
            });
        }
        // Items being rewielded (0x4655c0): never, here.
        0x32 => return Some(false),
        // NPC (operand 0) has met PC (operand 1) (0x4c0c40).
        0xa => {
            let npc = objects(run, block.operands[0], line, unsupported)?.first().copied();
            let pc = objects(run, block.operands[1], line, unsupported)?.first().copied();
            return Some(matches!((npc, pc), (Some(n), Some(p)) if ctx.has_met(n, p)));
        }
        // Has reputation N (0x4c1cb0: a PC's list).
        0x21 => {
            let rep = number(ctx, run, 1);
            return over_set(ctx, run, unsupported, &mut |ctx, o| u32::try_from(rep).is_ok_and(|r| ctx.pc_state(o).is_some_and(|s| s.reputations.contains(&r))));
        }
        // Has heard rumour N (0x4c58d0) / rumour N is quelled (0x4c5650).
        0x28 => {
            let rumour = number(ctx, run, 1);
            return over_set(ctx, run, unsupported, &mut |ctx, o| u32::try_from(rumour).is_ok_and(|r| ctx.pc_state(o).is_some_and(|s| s.rumours.contains(&r))));
        }
        0x29 => {
            let rumour = number(ctx, run, 0);
            return Some(u32::try_from(rumour).is_ok_and(|r| ctx.globals().quelled.contains(&r)));
        }
        // Mastered college N: all five of its spells.
        0x31 => {
            let college = number(ctx, run, 1);
            return over_set(ctx, run, unsupported, &mut |ctx, o| ctx.knows_spell(o, college * 5 + 4));
        }
        // Bless / curse N, bad associates: none exist yet (no blessings, curses or followers).
        8 | 9 | 0xb => return Some(false),
        0x30 => {
            let spell = number(ctx, run, 1);
            return over_set(ctx, run, unsupported, &mut |ctx, o| ctx.knows_spell(o, spell));
        }
        _ => {}
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
        // A global quest state (0x4c51a0: table 0x5ff418 from quest 1000) is N; nothing keeps
        // global quest states yet, so they are 0.
        7 => {
            let (_quest, state) = (value(0), value(1));
            state == 0
        }
        // Entry N of the switch table is on (0x4baba0).
        0x2e => {
            let n = value(0);
            u8::try_from(n).is_ok_and(|n| ctx.globals().switches.contains(&n))
        }
        _ => return None,
    })
}

/// An action (0x447220); `None` when it is not built.
fn action<C: ScriptContext>(ctx: &mut C, attachment: &mut Attachment, run: &mut Run<C::Object>, script: &Script, line: usize, block: &Block, unsupported: &mut Vec<Unsupported>) -> Option<Step> {
    let ops = block.operands;
    let depth = run.depth;
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
        // Call script N from line L for each triggerer and attachee (0x44b030): a record of its
        // own (not kept), the caller's point, no "other".
        0x7 => {
            let (number, start) = (value(ctx, attachment, run, 0), value(ctx, attachment, run, 1));
            let triggerers = objects(run, ops[2], line, unsupported)?;
            let attachees = objects(run, ops[3], line, unsupported)?;
            if number > 0 && depth < MAX_DEPTH {
                for &t in &triggerers {
                    for &a in &attachees {
                        let invocation = Invocation { attachee: a, triggerer: t, other: None, event: run.invocation.event, start_line: start.max(0) as usize };
                        self::run(ctx, &mut Attachment::new(0, 0, number as u32), invocation, depth + 1, unsupported);
                    }
                }
            }
            Step::Next
        }
        // The same in N seconds: a time event on the game clock, 8 game seconds a second.
        0x36 => {
            let (number, start, seconds) = (value(ctx, attachment, run, 0), value(ctx, attachment, run, 1), value(ctx, attachment, run, 4));
            let t = objects(run, ops[2], line, unsupported)?.first().copied();
            let a = objects(run, ops[3], line, unsupported)?.first().copied();
            if let (Some(t), Some(a), Ok(number)) = (t, a, u32::try_from(number)) {
                ctx.schedule(number, start, t, a, u64::try_from(seconds).unwrap_or(0) * 8000);
            }
            Step::Next
        }
        0x1d => {
            let npc = objects(run, ops[0], line, unsupported)?.first().copied();
            let pc = objects(run, ops[1], line, unsupported)?.first().copied();
            let v = match (npc, pc) {
                (Some(n), Some(p)) => ctx.reaction(n, p),
                _ => 50,
            };
            set(ctx, attachment, run, ops[2], v);
            Step::Next
        }
        // A stat (0x4b0490); Intelligence reads 1 for a dumb critter (0x45fc00).
        0x21 => {
            let stat = value(ctx, attachment, run, 0);
            let o = objects(run, ops[1], line, unsupported)?.first().copied();
            let v = o.map_or(0, |o| ctx.stat(o, stat));
            set(ctx, attachment, run, ops[2], v);
            Step::Next
        }
        // An area a PC knows now (0x4cafd0) / the area an object is in (0x4cb6a0).
        0x31 => {
            let area = value(ctx, attachment, run, 0);
            for o in objects(run, ops[1], line, unsupported)? {
                if let (Some(pc), Ok(a)) = (ctx.pc_state(o), u32::try_from(area)) {
                    pc.areas.insert(a);
                }
            }
            Step::Next
        }
        0x54 => {
            let o = objects(run, ops[0], line, unsupported)?.first().copied();
            let area = o.map_or(0, |o| ctx.area_of(o));
            set(ctx, attachment, run, ops[1], area);
            Step::Next
        }
        // Rumours: known by a PC (0x4c57e0) / quelled for everyone (0x4c5690).
        0x32 => {
            let rumour = value(ctx, attachment, run, 0);
            for o in objects(run, ops[1], line, unsupported)? {
                if let (Some(pc), Ok(r)) = (ctx.pc_state(o), u32::try_from(rumour)) {
                    pc.rumours.insert(r);
                }
            }
            Step::Next
        }
        0x33 => {
            if let Ok(r) = u32::try_from(value(ctx, attachment, run, 0)) {
                ctx.globals().quelled.insert(r);
            }
            Step::Next
        }
        // Reaction of NPC to PC: set (0x1e: by the difference) / adjust (0x1f) (0x4c0de0).
        0x1e | 0x1f => {
            let npc = objects(run, ops[0], line, unsupported)?.first().copied();
            let pc = objects(run, ops[1], line, unsupported)?.first().copied();
            let v = value(ctx, attachment, run, 2);
            if let (Some(n), Some(p)) = (npc, pc) {
                let delta = if block.kind == 0x1e { v - ctx.reaction(n, p) } else { v };
                ctx.change_reaction(n, p, delta);
            }
            Step::Next
        }
        // Distance between two objects.
        0x47 => {
            let a = objects(run, ops[0], line, unsupported)?.first().copied();
            let b = objects(run, ops[1], line, unsupported)?.first().copied();
            let d = match (a, b) {
                (Some(a), Some(b)) => ctx.distance(a, b),
                _ => 0,
            };
            set(ctx, attachment, run, ops[2], d);
            Step::Next
        }
        // Reputation given / taken away.
        0x48 | 0x49 => {
            let who = objects(run, ops[0], line, unsupported)?;
            let rep = value(ctx, attachment, run, 1);
            for o in who {
                if let (Some(pc), Ok(r)) = (ctx.pc_state(o), u32::try_from(rep)) {
                    if block.kind == 0x48 {
                        pc.reputations.insert(r);
                    } else {
                        pc.reputations.remove(&r);
                    }
                }
            }
            Step::Next
        }
        // Damage (0x4489f9): each object takes N of kind K (0, 2, 3 hit points, 1 poison, 4
        // fatigue); no attacker.
        0x3c => {
            let (amount, kind) = (value(ctx, attachment, run, 1), value(ctx, attachment, run, 2));
            for o in objects(run, ops[0], line, unsupported)? {
                ctx.damage(o, amount, kind);
            }
            Step::Next
        }
        // A spell with no caster on one object (0x448ab7).
        0x3d => {
            let spell = value(ctx, attachment, run, 0);
            if let Some(target) = objects(run, ops[1], line, unsupported)?.first().copied() {
                ctx.cast_on(spell, target);
            }
            Step::Next
        }
        // The script's own object becomes prototype N (0x447e89).
        0x28 => {
            let prototype = value(ctx, attachment, run, 0);
            ctx.transform(run.invocation.attachee, prototype);
            Step::Next
        }
        // A plaque (0x44a15d, `mes/gameplaque.mes`) / a book (0x448ba4, `mes/gamebook.mes`) from
        // line N, to one player.
        0x72 | 0x40 => {
            let line_id = value(ctx, attachment, run, 0);
            if let Some(to) = objects(run, ops[1], line, unsupported)?.first().copied() {
                ctx.show_text(if block.kind == 0x72 { 6 } else { 0 }, line_id, to);
            }
            Step::Next
        }
        // The art number of each object (0x448947).
        0x3b => {
            let n = value(ctx, attachment, run, 1);
            for o in objects(run, ops[0], line, unsupported)? {
                ctx.set_art_number(o, n);
            }
            Step::Next
        }
        // Skill N's level (0x448374).
        0x2f => {
            let k = value(ctx, attachment, run, 0);
            let o = objects(run, ops[1], line, unsupported)?.first().copied();
            let v = o.map_or(0, |o| ctx.skill_level(o, k));
            set(ctx, attachment, run, ops[2], v);
            Step::Next
        }
        // Destroy one object (0x448d19).
        0x44 => {
            if let Some(o) = objects(run, ops[0], line, unsupported)?.first().copied() {
                ctx.destroy(o);
            }
            Step::Next
        }
        // Flip entry N of the switch table (0x44a067).
        0x6e => {
            if let Ok(n) = u8::try_from(value(ctx, attachment, run, 0)) {
                let switches = &mut ctx.globals().switches;
                if !switches.remove(&n) {
                    switches.insert(n);
                }
            }
            Step::Next
        }
        // Switch each object off or on (0x38: object flag 0x2, "off"; 0x39: 0x400000).
        0x38 | 0x39 => {
            let mask = if block.kind == 0x38 { 0x2 } else { 0x40_0000 };
            for o in objects(run, ops[0], line, unsupported)? {
                ctx.toggle_flags(o, mask);
            }
            Step::Next
        }
        // Magic/tech against a target (0x461620): N less the percent the item's magick or tech
        // loses on a critter of the other kind, `N × (100 - r) / 100`.
        0x50 => {
            let n = value(ctx, attachment, run, 0);
            let item = objects(run, ops[1], line, unsupported)?.first().copied();
            let user = objects(run, ops[2], line, unsupported)?.first().copied();
            let target = objects(run, ops[3], line, unsupported)?.first().copied();
            let r = match (item, user, target) {
                (Some(item), Some(user), Some(target)) if [user, target].into_iter().all(|o| matches!(ctx.object_type(o), 15 | 16)) => {
                    let k = ctx.object_int(item, "item_magic_tech_complexity").unwrap_or(0);
                    magic_tech_resistance(k, ctx.stat(user, 0x10), ctx.stat(target, 0x10))
                }
                _ => 0,
            };
            set(ctx, attachment, run, ops[4], n * (100 - r) / 100);
            Step::Next
        }
        // Give an effect (0x4e9f70) for a cause / take one (0x4ea100), to each object.
        0x4d | 0x4e => {
            let id = value(ctx, attachment, run, 1);
            let cause = value(ctx, attachment, run, 2);
            for o in objects(run, ops[0], line, unsupported)? {
                if block.kind == 0x4d {
                    ctx.give_effect(o, id, cause);
                } else {
                    ctx.remove_effect(o, id);
                }
            }
            Step::Next
        }
        // Sound (0x52), eye candy (0x78) and slides (0x83): not shown or played yet; the script
        // goes on, as the original's does.
        0x52 | 0x78 | 0x83 => Step::Next,
        // Magic/tech adjust (0x461540): N scaled by how well the user's aptitude meets the
        // item's complexity, rounded: `(F × N + 50) / 100`.
        0x5b => {
            let n = value(ctx, attachment, run, 0);
            let item = objects(run, ops[1], line, unsupported)?.first().copied();
            let user = objects(run, ops[2], line, unsupported)?.first().copied();
            let complexity = item.and_then(|i| ctx.object_int(i, "item_magic_tech_complexity")).unwrap_or(0);
            let aptitude = user.map_or(0, |u| ctx.stat(u, 0x10));
            let critter = user.is_some_and(|u| matches!(ctx.object_type(u), 15 | 16));
            let v = (magic_tech_factor(complexity, aptitude, critter) * n + 50) / 100;
            set(ctx, attachment, run, ops[3], v);
            Step::Next
        }
        // Set a PC's quest (0x4c4d20): only upwards, and never from completed (4), other (5) or
        // botched (6).
        0x10 => {
            let who = objects(run, ops[0], line, unsupported)?;
            let (quest, state) = (value(ctx, attachment, run, 1), value(ctx, attachment, run, 2));
            for o in who {
                if let Some(pc) = ctx.pc_state(o) {
                    let now = pc.quest(quest);
                    if !(4..=6).contains(&now) && now < state && let Ok(q) = u32::try_from(quest) {
                        pc.quests.insert(q, state);
                    }
                }
            }
            Step::Next
        }
        // Float a line of the script's dialogue over each object (0x44b1f0).
        0x17 => {
            let number = value(ctx, attachment, run, 0);
            for o in objects(run, ops[1], line, unsupported)? {
                if let Some(text) = ctx.dialog_text(attachment.number, number, run.invocation.triggerer) {
                    ctx.float_text(o, text);
                }
            }
            Step::Next
        }
        0x22 => {
            let o = objects(run, ops[0], line, unsupported)?.first().copied();
            let kind = o.map_or(0, |o| ctx.object_type(o));
            set(ctx, attachment, run, ops[1], kind);
            Step::Next
        }
        0x23 => {
            let n = value(ctx, attachment, run, 1);
            if let Some(o) = objects(run, ops[0], line, unsupported)?.first().copied() {
                ctx.change_gold(o, n);
            }
            Step::Next
        }
        0x29 => {
            let name = value(ctx, attachment, run, 0);
            let from = objects(run, ops[1], line, unsupported)?.first().copied();
            let to = objects(run, ops[2], line, unsupported)?.first().copied();
            if let (Some(from), Some(to)) = (from, to) {
                ctx.transfer_item_named(name, from, to);
            }
            Step::Next
        }
        0x42 => {
            let basic = value(ctx, attachment, run, 0);
            if let Some(o) = objects(run, ops[1], line, unsupported)?.first().copied() {
                ctx.create_item(basic, o);
            }
            Step::Next
        }
        // Call another script (0x44b170 → 0x441980): the object's at point P from line L, with
        // triggerer T; its result is ignored, its attachment kept.
        0x51 => {
            let owner = objects(run, ops[0], line, unsupported)?.first().copied();
            let (point, start) = (value(ctx, attachment, run, 1), value(ctx, attachment, run, 2));
            let triggerer = objects(run, ops[3], line, unsupported)?.first().copied();
            if let (Some(owner), Some(triggerer), Ok(point)) = (owner, triggerer, u32::try_from(point)) {
                let mut called = ctx.attachment(owner, point);
                if called.number != 0 && depth < MAX_DEPTH {
                    let invocation = Invocation { attachee: owner, triggerer, other: None, event: point, start_line: start.max(0) as usize };
                    self::run(ctx, &mut called, invocation, depth + 1, unsupported);
                    ctx.set_attachment(owner, point, called);
                }
            }
            Step::Next
        }
        // Change a script (0x63): the running one's when it is this object and point.
        0x63 => {
            let owner = objects(run, ops[0], line, unsupported)?.first().copied();
            let (point, number) = (value(ctx, attachment, run, 1), value(ctx, attachment, run, 2));
            if let (Some(owner), Ok(point)) = (owner, u32::try_from(point)) {
                let number = number.max(0) as u32;
                if owner == run.invocation.attachee && point == run.invocation.event {
                    run.replace = Some(number);
                } else {
                    let mut changed = ctx.attachment(owner, point);
                    changed.number = number;
                    ctx.set_attachment(owner, point, changed);
                }
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
        // Everyone / anyone in the vicinity (0x44b4e0 cases 8/9): critters around the attachee.
        kind if NearKind::of_set(kind).is_some() => run.near.get(&NearKind::of_set(kind)?).cloned().unwrap_or_default(),
        0x15 => run.player.into_iter().collect(),
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

/// The percent an item's magick or tech loses against a critter of the other kind (0x461620):
/// `k` the item's adjustment (field 0x60: above 0 magick, below 0 tech), `user` and `target`
/// the two critters' aptitudes (stat 0x10). A magick item on a technologist (`target` below 0)
/// loses the target's whole aptitude when the user is a technologist too, else
/// `(100 - user) × -target / 100`; a tech item on a mage, the same the other way round.
pub fn magic_tech_resistance(k: i32, user: i32, target: i32) -> i32 {
    match () {
        _ if k > 0 && target < 0 => {
            if user < 0 { -target } else { (100 - user) * target / -100 }
        }
        _ if k < 0 && target > 0 => {
            if user > 0 { target } else { (user + 100) * target / 100 }
        }
        _ => 0,
    }
}

/// How well an item of `complexity` works for a user of `aptitude`, in percent (0x461540 /
/// 0x4614a0): 100 for no complexity; else `100 × effective / complexity`, the effective part
/// being half of aptitude plus complexity, kept between 0 and the complexity. A user that is no
/// critter gets the whole complexity.
pub fn magic_tech_factor(complexity: i32, aptitude: i32, critter: bool) -> i32 {
    if complexity == 0 {
        return 100;
    }
    let effective = if !critter {
        complexity
    } else {
        let m = (aptitude + complexity) / 2;
        if complexity >= 0 { m.clamp(0, complexity) } else { m.clamp(complexity, 0) }
    };
    100 * effective / complexity
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
        /// Attachments by object and event.
        pub attachments: BTreeMap<(u32, u32), Attachment>,
        /// Objects near each object.
        pub near: BTreeMap<u32, Vec<u32>>,
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
        fn attachment(&mut self, object: u32, event: u32) -> Attachment {
            self.attachments.get(&(object, event)).copied().unwrap_or_default()
        }
        fn set_attachment(&mut self, object: u32, event: u32, attachment: Attachment) {
            self.attachments.insert((object, event), attachment);
        }
        fn near(&mut self, centre: u32, _kind: NearKind) -> Vec<u32> {
            self.near.get(&centre).cloned().unwrap_or_default()
        }
        fn player(&mut self) -> Option<u32> {
            Some(1)
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

    #[test]
    fn scripts_call_and_change_scripts_and_name_the_vicinity_and_the_player() {
        // Script 100 calls object 50's event 3 script (200) from line 1 with the player as
        // triggerer, changes 50's event 4 script to 300, and counts everyone near (set 8).
        let lines = vec![
            always(block(0x51, &[op(0xc, 0), n(3), n(1), op(0x15, 0)])),
            always(block(0x63, &[op(0xc, 0), n(4), n(300)])),
            line(block(0x2b, &[n(9)]), block(0, &[]), block(0x12, &[op(8, 0)])),
            always(block(0xb, &[op(2, 0), op(2, 0), n(1)])),
            always(block(0x13, &[])),
            always(block(0xa, &[op(1, 6), op(2, 0)])),
            always(block(1, &[])),
        ];
        let mut w = world(lines, 0);
        // Script 200: line 0 sets global flag 1 (skipped by starting at 1); line 1 the triggerer's PC flag 7.
        let called = vec![always(block(0x64, &[n(1)])), always(block(0xa, &[op(6, 7), n(1)])), always(block(5, &[]))];
        w.scripts.insert(200, Arc::new(Script { description: String::new(), flags: 0, lines: called }));
        w.attachments.insert((50, 3), Attachment::new(0, 0, 200));
        w.attachments.insert((50, 4), Attachment::new(0, 0, 250));
        w.near.insert(40, vec![40, 1, 2]);
        let mut attachment = Attachment::new(0, 0, 100);
        let invocation = Invocation { attachee: 40, triggerer: 2, other: Some(50), event: 1, start_line: 0 };
        let run = run_hook(&mut w, &mut attachment, invocation);
        assert!(run.unsupported.is_empty(), "{run:?}");
        assert!(!w.globals.flag(1), "called from line 1");
        assert!(w.pcs[&1].flags.contains(&7), "the player (set 0x15) was its triggerer");
        assert_eq!(w.attachments[&(50, 3)].number, 0, "the called script removed itself (action 5)");
        assert_eq!(w.attachments[&(50, 4)].number, 300);
        assert_eq!(w.globals.var(6), 3, "three in the vicinity");
    }

    #[test]
    fn magic_tech_and_calls_by_number_and_rumours() {
        assert_eq!(magic_tech_factor(0, -50, true), 100, "no complexity");
        assert_eq!(magic_tech_factor(40, 40, true), 100, "aptitude at the complexity");
        assert_eq!(magic_tech_factor(40, 0, true), 50, "half without aptitude");
        assert_eq!(magic_tech_factor(40, -60, true), 0, "tech against magic");
        assert_eq!(magic_tech_factor(-40, -40, true), 100, "tech with tech");
        assert_eq!(magic_tech_factor(40, -60, false), 100, "not a critter: all of it");
        // 0x461620: magick on a technologist, tech on a mage.
        assert_eq!(magic_tech_resistance(40, 0, -50), 50, "a neutral user: (100 - 0) x 50 / 100");
        assert_eq!(magic_tech_resistance(40, 60, -50), 20, "a mage user: (100 - 60) x 50 / 100");
        assert_eq!(magic_tech_resistance(40, -10, -50), 50, "a technologist user: the target's whole aptitude");
        assert_eq!(magic_tech_resistance(-40, -60, 50), 20, "tech: (100 - 60) x 50 / 100");
        assert_eq!(magic_tech_resistance(-40, 30, 50), 50);
        assert_eq!(magic_tech_resistance(40, 0, 50), 0, "magick on a mage: nothing lost");
        assert_eq!(magic_tech_resistance(0, 0, -50), 0, "no adjustment");
        // Script 100: calls 200 from line 1 for triggerer 1 on attachee 50, adjusts 10 by item
        // 60 (no complexity here) into global 3, sets rumour 1040 for PC 1, quells 1041, tests
        // both and reputation 9 into flags 11-13.
        let lines = vec![
            always(block(7, &[n(200), n(1), op(0, 0), op(0xc, 0)])),
            always(block(0x5b, &[n(10), op(0xc, 0), op(0, 0), op(1, 3)])),
            always(block(0x32, &[n(1040), op(0, 0)])),
            always(block(0x33, &[n(1041)])),
            line(block(0x28, &[op(0, 0), n(1040)]), block(0x64, &[n(11)]), block(0, &[])),
            line(block(0x29, &[n(1041)]), block(0x64, &[n(12)]), block(0, &[])),
            line(block(0x21, &[op(0, 0), n(9)]), block(0x64, &[n(13)]), block(0, &[])),
            always(block(1, &[])),
        ];
        let mut w = world(lines, 0);
        w.scripts.insert(200, Arc::new(Script { description: String::new(), flags: 0, lines: vec![always(block(0x64, &[n(1)])), always(block(0x64, &[n(2)]))] }));
        let mut attachment = Attachment::new(0, 0, 100);
        let invocation = Invocation { attachee: 40, triggerer: 1, other: Some(50), event: 1, start_line: 0 };
        let run = run_hook(&mut w, &mut attachment, invocation);
        assert!(run.unsupported.is_empty(), "{run:?}");
        assert!(!w.globals.flag(1) && w.globals.flag(2), "script 200 from line 1");
        assert_eq!(w.globals.var(3), 10);
        assert!(w.pcs[&1].rumours.contains(&1040));
        assert!(w.globals.flag(11) && w.globals.flag(12) && !w.globals.flag(13));
    }

    #[test]
    fn reputations_come_and_go_and_sounds_and_candy_pass() {
        let lines = vec![
            always(block(0x48, &[op(0, 0), n(1003)])),
            always(block(0x48, &[op(0, 0), n(1004)])),
            always(block(0x49, &[op(0, 0), n(1003)])),
            always(block(0x52, &[n(4000)])),
            always(block(0x78, &[n(7), op(1, 0)])),
            always(block(0x4e, &[op(0, 0), n(3)])),
            line(block(8, &[op(0, 0), n(1)]), block(0x64, &[n(1)]), block(0, &[])),
            always(block(1, &[])),
        ];
        let mut w = world(lines, 0);
        let run = invoke(&mut w, &mut Attachment::new(0, 0, 100));
        assert!(run.unsupported.is_empty(), "{run:?}");
        assert_eq!(w.pcs[&1].reputations.iter().copied().collect::<Vec<_>>(), [1004]);
        assert!(!w.globals.flag(1), "no blessings yet");
    }
}
