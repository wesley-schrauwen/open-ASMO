# The script engine

The original's scripts decide what objects do when used, talked to, entered, killed and so on:
inn beds, quests, dialogue starts, traps, story state. This plan copies its engine
(Arcanum.exe 0x441980 hook, 0x4449b0 runner, 0x4451b0 conditions, 0x447220 actions) into
`arcanum_sim`, running on the host. Findings come from three Sonnet agents (2026-11-23), cross-checked where they disagreed.

## Multiplayer (owner's requirement)

The engine must work in multiplayer. The original runs every script on the host (and in single
player). A client runs only scripts 997 and 999 (end slides, credits) and treats every other
hook as "skip the default" (0x4449b0's gate on 0x62ac6c). Global state changes go through
helpers that apply them on the host and broadcast them (message 0x7c).

Here:
- **Host only.** Scripts run in `arcanum_sim`. Clients never run them; they see the results as
  deltas, as they do for doors and items.
- **State where it belongs:**
  - global variables, global flags and the story state go in the `WorldSave`;
  - an attachment's flags and counters go with the object (the save's sector changes for map
    objects, the PC's own record for a PC);
  - PC variables and flags go with each PC.
- **The triggerer** is the PC of the player who acted, never "the local player". Object set
  0x15 ("the local player", 0x40da50) needs a rule: the triggerer when there is one, else the
  party leader (builder's call, to confirm).
- **Effects addressed to players:**
  - float text and dialogue go to the triggering player, or to the players who can see it;
  - fades and teleports (actions 0x66/0x67) are skipped in multiplayer in the original, and
    here they move only the PCs involved.
- **Randomness** comes from the host's `HostRng`.

## File format (`arcanum_formats::script`, done)

- **Finding a script:** script `n` is the `scr/*.scr` file whose name starts with `n`'s digits
  (`01729Bed_Use.scr`). Numbers below 1000 go in a separate table (0x4cece0). Duplicates are
  logged, and the first wins.
- **Header** (0x40 bytes):
  - 8 ignored bytes;
  - a 0x28-byte description;
  - flags at 0x30: bit 4 runs the script once; bits 8 and 0x10 concern dialogue (below);
  - the line count at 0x34;
  - the capacity at 0x38;
  - a stale pointer at 0x3c.
- **Lines:** each is 0x84 bytes: a condition block, a then block and an else block, 0x2c bytes
  each. A block is a type (`u32`), eight operand tags (`i8` at +4..+0xb) and eight operand
  values (`i32` from +0xc). Operands a type does not use are garbage. Operands are read in
  order (action 0xa: operand 0 := operand 1; 0xb: operand 0 := operand 1 + operand 2). An
  object operand's tag is an object set kind.
- **Checked:** all 2487 scripts in the archives parse exactly. Condition types run 0-52,
  action types 0-141.

## Running a script (0x441980, 0x4449b0)

- **The hook** `(triggerer, attachee, other, event)`:
  - it reads the attachee's `scripts_idx` (field 0x20) entry for the event: `{flags, counters, number}`;
  - with no script there it returns 1, "run the default";
  - with `event == 1` it first runs two checks (0x4bbfe0, 0x4bbe40; likely theft and witnesses, not decoded).
- **The runner** steps lines from a start line, at most 1000 steps. A line evaluates its
  condition and runs the then or else action, whose result decides what happens next:
  - -1: the next line; past the last line, end as -2;
  - -2 (action 1): end and skip the default; the hook returns 0;
  - -3 (action 2): end and run the default; the hook returns 1;
  - -4: end and run the default, without the run-once detach;
  - 0 or more: go to that line (action 3).
  - Hitting the step limit returns 0.
  - With flag 4, a -2 or -3 end clears the attachment's script number.
  - Actions 5 and 6 replace the attachment's script number after the run.
- **NPC dialogue (event 9) special cases** (0x4449b0), before running:
  - an NPC with flag 0x80000 in field 0x118 returns 1;
  - an NPC for which 0x45d8d0 is true (probably dead or unconscious) returns 1 unless script flag 8 is set;
  - one for which 0x4af210 is true returns 1 unless script flag 0x10 is set.
- **Conditions over object sets** count the members that pass. "Any" set kinds (3, 5, 7, 9,
  0xe, 0x10, 0x12, 0x14, 0x17) need one; the others need all, and at least one member.

## Operands

- **Scalar tags** (0x44ba70; verified against the jump table 0x44bb34):
  - 0: a counter byte of the attachment (0-3);
  - 1: global variable (2000);
  - 2: a variable local to the run (10);
  - 3: a number;
  - 4: global flag (3200);
  - 5: a PC variable (field 0x111);
  - 6: a PC flag (field 0x110);
  - for 5 and 6, the value's high 16 bits are an object set and the low 16 the index;
  - any other tag is a number.
  - The two agents disagreed on 1/4/5/6. The table settles it: 1 and 4 call 0x444c80 and
    0x444cf0 (globals), 5 and 6 call 0x444db0 and 0x444e30 (the PC's).
- **Object sets** (0x44b4e0):
  - 0 attachee; 1 triggerer; 0xa the loop's object; 0xb a local object variable;
  - 0xc the "other" object; 0x15 the local player;
  - 2/3 the attachee's followers; 0xd/0xe the followers and the attachee;
  - 8/9 critters near the triggerer; 0xf/0x10 scenery; 0x11/0x12 containers; 0x13/0x14
    portals; 0x16/0x17 items;
  - 4-7 not decoded.
- **Globals** (0x4446e0 init; saved by 0x4448d0 / loaded by 0x444930): 2000 variables, 100
  dwords of flags, the story state (0x5e2fa4, which only rises).

## Conditions (table 0x44714c, 53) and actions (table 0x44adbc, 141)

Decoded so far, with confidence; the rest are named by the helper they call.
- **Conditions:**
  - 0 always; 1 daytime;
  - 2 has gold ≥ V; 3 local flag; 4 V == V; 5 V ≤ V;
  - 6/7 quest state (per object / global); 8/9 has curse / blessing; 0xa reaction;
  - 0x11 has an item of a prototype; 0x12 leader is; 0x16 dead or down; 0x17 hit point ratio;
  - 0x20 at a location; 0x21 has reputation; 0x22 distance ≤ V;
  - 0x2b global flag set;
  - others named by their helpers.
- **Actions:**
  - control: 1 end and skip the default; 2 end and run the default; 3 go to a line;
    4 dialogue; 5 remove this script; 6 change it; 7 call a script;
  - flags and arithmetic: 8/9 set/clear a local flag; 0xa assign; 0xb-0xe add, subtract,
    multiply, divide;
  - quest state and loops: 0x10/0x11 set quest state; 0x12-0x14 loop over a set (for, end, break);
  - followers: 0x15/0x16 add/remove a follower;
  - curses and blessings: 0x19-0x1c add/remove;
  - reaction and stats: 0x1d-0x1f reaction; 0x21 a stat;
  - gold and items: 0x23 give/take gold; 0x25 random; 0x29 move an item;
  - story state and travel: 0x2a/0x2b story state; 0x2c teleport; 0x36/0x37 timed events;
  - float text: 0x40/0x41/0x72; 0x4c damage;
  - attachments and global flags: 0x63 attach a script; 0x64/0x65 set/clear a global flag;
  - fades: 0x66/0x67 fade and teleport.

## Steps (one PR each)

1. **The reader** (`arcanum_formats::script`, `ScriptIndex`) and this plan. Done.
2. **The runner on the host** (`arcanum_sim::script`):
   - the hook, lines, results, the step limit, goto, run-once;
   - the scalar operands and the object sets for attachee, triggerer and other;
   - global variables, global flags and the story state in the `WorldSave`;
   - attachment flags and counters saved with their object;
   - the control, flag, arithmetic and comparison actions and conditions.
   - Tested on synthetic scripts and on real ones read from the archives.
3. **Use** (event 1) on doors, containers and scenery through the runner, so the default use
   can be skipped. Beds: the sleep window's bed path (`plans/world_milestone.md` → "Beds wait
   for scripts"), with occupancy and double healing.
4. **Gold and items:** conditions 2 and 0x11; actions 0x23, 0x29 and 0x42. These are the inn
   beds' needs; check with `02429Black_Root_Inn_Bed_Use.scr`.
5. **Float text and messages** (0x40/0x41/0x72) as deltas to the right players.
6. Later: dialogue (4, 0x30, event 9), quest state, reputation, followers, timed events,
   teleports, combat-related actions, the remaining conditions.

## Open

- What decides the start line (B+4) besides 0. The theft checks for event 1. Attach point
  names (no table in the exe; 9 is dialogue).
- Script flags other than 4, 8 and 0x10. The 8 ignored header bytes.
