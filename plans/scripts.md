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
- **Big moves wait for everyone** (owner's decision, 2026-11-23):
  - What waits: a teleport, a map change (stairs up or down, entering a building such as a
    cathedral), anything instanced, or another sudden big movement. Every player sees a
    popup naming who caused it, and it happens only once all have ticked it.
  - Scripts: teleports (0x2c, 0x66) and their fades (0x67).
  - Not scripts: the map's jump points and stairs; see `plans/multiplayer.md`.
  - What does not wait: quests, the story state, reputation, gold, items, float text and
    dialogue.
  - For the runner: a run must be able to **pause** at a teleport and **resume** after the
    vote. Its state (line, locals, local objects, loop, pending script replacement) must be a
    value the host keeps across ticks, and in the save, not a stack frame.
  - Decided (2026-11-24, `plans/multiplayer.md`): the clock runs and all can act while waiting;
    anyone can decline, which cancels the move; leavers drop out; no timeout. Still open: combat
    during a vote.

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
  - 0 triggerer; 1 attachee (corrected 2026-11-24: 0x44b4e0 reads +0x8 and +0x10 of the run's
    objects, which 0x441980 fills with its first argument, the triggerer, and its second, the
    attachee whose `scripts_idx` it reads; the teleports' set 0 moving the PC confirms it);
  - 0xa the loop's object; 0xb a local object variable; 0xc the "other" object; 0x15 the
    local player;
  - 2/3 the triggerer's followers (0x4411c0 on +0x8); 0xd/0xe the followers and the triggerer;
  - 8/9 critters near the triggerer (to check against +0x8 as well); 0xf/0x10 scenery; 0x11/0x12 containers; 0x13/0x14
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
2. **The runner on the host** (`arcanum_sim::script`). Done.
   - `run_hook` runs an `Attachment` for an `Invocation` (attachee, triggerer, other, event,
     start line) through a `ScriptContext` the host provides, and says whether the default
     runs.
   - Built:
     - conditions 0, 1, 3, 4, 5 and 0x2b;
     - actions 1-3, 5, 6, 8, 9, 0xa-0xf, 0x12-0x14, 0x25, 0x2a, 0x2b, 0x64 and 0x65;
     - object sets 0, 1, 0xa, 0xb and 0xc;
     - all scalar operands.
   - Global state is saved in the `WorldSave` (`scripts`), a PC's in its `PlayerSave`
     (`script_state`).
   - `ScriptLibrary` indexes `scr/` and parses on first use.
   - Loop break follows 0x44bc60: it goes past the first line from the loop's start whose
     condition is 0 and whose then action is 0x13. Random is inclusive (0x4cef00).
   - **Builder's call:** a condition, action or object set not built yet stops the script
     and runs the default, and the runner reports it, rather than half-running it.
   - **Real data:** from their first line, 459 of the 2399 scripts run to an end. The rest
     stop at, most often:
     - action 4, dialogue (262 scripts);
     - action 0x2c, teleport (261);
     - condition 6, quest state (139);
     - actions 0x22 and 0x51 (105 each);
     - action 0x18 (89);
     - condition 0x14 (80);
     - object set 8, critters near (67).
3. **Use on doors and containers** (`arcanum_sim::script_host`). Done.
   - Attachments are read from `scripts_idx`, the object's own or its prototype's. It is a
     sparse array: bit `e` of the bitset marks event `e`, with one 12-byte `{flags, counters,
     number}` entry per set bit, in order.
   - A changed attachment is kept in the sector's changes (`object_scripts`, `mobile_scripts`)
     and saved.
   - `ScriptHost::hook` runs the script with the acting player's PC as the triggerer. Doors
     (`use_portals`) and containers (`pick_up_items`) run it before their default.
   - **Traps** (scripts 30000-30007) are sprung by 0x4bcb70 before any line, and the use does
     not happen. Not built: they run the default and are reported. Traps need their own step
     (Disarm Traps, damage).
   - **Start map** (around the start): 20 containers and 1 piece of scenery carry use
     scripts. Most are traps, plus 1180 ("Grave_Container_Use") and others not built yet,
     which run the default. 1621 ("Search the bed") keeps its bed shut until global flag 2273
     is set.
3b. **Beds and scenery use.** Done.
   - Using scenery (the same `UsePortal` request as doors: the PC walks there first) runs its
     use script.
   - A bed (art category 22, 0x428c74) asks for the sleep window on it instead (`BedRequests`,
     `arcanum_sim::rest`):
     - no town map check;
     - the bed's use script running the default refuses with `SleepUI.mes` 8;
     - another PC resting in it refuses with 10;
     - otherwise the window opens for sleeping, and each hour heals twice.
   - The host keeps what the window was opened with (`RestOffer`).
   - **Client:** a click on a bed, or on scenery with a use script, walks there and uses it.
     Other scenery lets the click through to the ground (builder's call, so trees do not swallow
     walking).
   - **Data:** all beds are sector objects. The start map has 271 beds, 47 of them with a use
     script; Bates Mansion's beds all have one; most other maps have none (so their beds
     refuse).
   - **Not built:** hiding the PC in the bed and moving it off the bed's tile (0x45f2d0); NPCs
     in beds.
4. **Messages and time** (inn beds). Done.
   - Action 0x18 (0x44b390) shows line `operand 0` of the script's own dialogue file in the
     triggerer's message box. The file is `dlg/` beside `scr/`, with the same name and a
     `.dlg` extension (0x4cee60). In multiplayer it goes to that player only (0x4a2b10).
     Here it is `LedgerDelta::ScriptMessage`, in the text for the PC's gender.
   - `.dlg` files (`arcanum_formats::dialog`) have seven groups per entry: number, male
     text, female text, intelligence, test, response, result.
   - Actions 0x60 (day + 1), 0x61 (hour), 0x62 (minute) and 0x69 (hours since the start:
     `days * 24 + hour`, 0x45a890).
   - **Inn beds** (`01990Inn_Bed_Use.scr`):
     - the bed's counter 0 picks the inn;
     - global variables 1043-1052 hold the hour each inn's room is paid until;
     - "now ≤ paid" sleeps;
     - otherwise the script shows "You must pay the Inn Keeper to use this bed." (or "...
       again.") and the bed refuses.
     - Paying happens in the innkeeper's dialogue (not built).
   - **Real data:** 543 of the 2399 scripts now run to an end from their first line.
4b. **Teleports** (`arcanum_sim::big_move`). Same map built.
   - **The original:** action 0x2c (0x4481a1): objects (operand 0), map (a `Rules/MapList.mes`
     id, 5000 the start map), x, y. It queues the move (0x4d3380) and the run goes on; the move
     happens after (0x4d3330 → 0x4d3460 / 0x4d39a0): followers come along to the same tile, the
     tile is used as given, the camera recentres. 0x66 (0x449a85) is the same with a fade, sound
     and delay first (operands 0-2; objects 3, map 4, x 5, y 6); 0x67 a fade alone. 0x66 and 0x67
     are skipped in multiplayer (`[0x62ac6c] & 1`).
   - **Data:** 457 teleport actions in 357 scripts; the object is set 0 (the triggerer) in 440,
     1 in 8, set 21 in 9. Maps 5000-5075 and more; 5000 in 157.
   - **Here:** the run does not wait (as the original). A teleport of a PC within the loaded map
     is put to every player's vote (`plans/multiplayer.md`); with one player it happens at once.
     The PC jumps (`Teleported`), standing; its position is saved.
   - **Found on the way:** object sets 0 and 1 were the wrong way round (0 is the triggerer).
   - **Other maps** (built next, `feat/map-switch`): the whole party goes, after the same vote.
     The old map's changes go into `WorldSave::other_maps`, the new map's come out; the old map's
     NPCs despawn, the new map opens around the tile and its NPCs spawn as its sectors load.
     Clients get `EnterMap`, the map's changes and `Teleported`, and clear the old map's
     sprites. Talks end; barter and loot windows on the old map close on the host.
     Real data: a teleport to map 5001 opens `BessieToonesMine-fixed` with 16 NPCs round the
     tile.
   - **Not built:** teleporting NPCs and objects; followers; fades and delays (0x66 teleports
     without them, 0x67 does nothing); votes in the save; the client's open dialogue, barter
     and loot windows are not closed by the switch.
   - **Jump points** (built next, `feat/jump-points`): `maps/<map>/map.jmp` (loader 0x4e30a0 /
     0x4e3800): a `u32` count, then 32-byte entries (flags, pad, source x/y, map, pad, destination
     x/y); the map is `id - 4999`, -1 this map; the flags are never read. A PC placed on a
     source tile (0x444270 → 0x4101d0 → 0x4e3450; each step ending there) queues a teleport
     (0x4d3380, no fade); NPCs and followers do not. Here it goes to the same vote. Data: 45 jump
     points on 17 maps, 22 to the same map; every destination map is listed. Stairs and doors
     to other maps are script teleports on their "use" (the `_TEL` scripts), built above.
4c. **Quests, gold, items, calls and the vicinity** (built, `feat/script-gaps`; names from
   `semes/condition.mes` and `semes/action.mes`, whose numbers are the types).
   - **Conditions** (0x4451b0; over a set: count 0 is false, else all must pass, or one for
     the "anyone" sets):
     - 2 gold at least (0x464780); 6 a PC's quest state exactly (0x4c4cb0: botched, flag
       0x100, reads 6; not a PC, the global quest state: not built, 0); 0x11 carries an item
       whose `name` is N (0x462480); 0x14 is named N and not `flags` 0x8000000; 0x16 dead
       (0x45d8d0: hit points below 1; nothing takes damage yet, so never); 0x2f identified
       (0x464370); 0x32 items being rewielded (never here).
   - **Actions** (0x447226):
     - 0x10 set a PC's quest (0x4c4d20): only upwards, never from 4, 5 or 6. Not built: the
       global quest states (2 → 4/5/6), the quest XP at 4, the giver's reaction (at least 41 at
       2, +10 at 4), the timestamp.
     - 0x17 float a line of the script's dialogue over each object (0x44b1f0), to every player;
       the original's duration and "a PC near" check are not built.
     - 0x22 object type (raw `obj_f_type`); 0x23 gold (0x464830: a take larger than the gold
       does nothing); 0x29 move the first item named N (0x4617f0; no room, it stays); 0x42 make
       an item of basic prototype N inside (no room: the original leaves it on the floor; here
       it is not made).
     - 0x51 call the script at another object's point P from line L with triggerer T (0x44b170
       → 0x441980; result ignored, attachment kept; nested up to the runner's depth); 0x63
       change a script (the running one's number if it is the same object and point).
   - **Sets:** 8/9 everyone / anyone in the vicinity: PCs and NPCs round the attachee
     (0x440fc0, mask 0x18000; the original's area is a view-sized rectangle, here 10 tiles);
     0x15 the player: the acting player's PC.
   - **Host:** scripts change PCs' inventories in the save; `sync_script_inventories` copies
     them to the PCs and sends them. NPC and container inventories are kept in the sector's
     changes.
   - **Real data:** from their first line, 1731 of 2399 scripts now run to an end (1085
     before). Next most common stops: action 0x5b (119, magic/tech adjust), 0x7 (35), 0x1d (24,
     reaction), 0x21 (21), 0x36 (20).
4d. **Calls, timers, stats, rumours, the rest of the vicinity** (built, `feat/script-gaps-2`).
   - **Actions:** 0x7 call script N from line L for each triggerer × attachee (0x44b030: a
     record of its own, not kept; the caller's point); 0x36 the same once N × 8 game seconds
     have passed (a time event of type 4 on the game clock; here `TimedScripts`, not in the save
     yet; one triggerer and attachee); 0x1d store the reaction (0x4c0cc0); 0x21 store an
     effective stat (0x4b0490; NPCs: their base stats); 0x32 a PC knows rumour N (0x4c57e0);
     0x33 rumour N quelled for everyone (0x4c5690, bits 0x6876c8); 0x5b magic/tech adjust
     (0x461540: `(F × N + 50) / 100`, F = 100 × effective / complexity, effective = half of
     aptitude plus complexity, kept between 0 and the complexity).
   - **Conditions:** 0xa met before (0x4c0c40); 0x21 has reputation; 0x28 heard rumour; 0x29
     rumour quelled; 0x30 knows spell (college level above the spell's place).
   - **Sets:** 0xf/0x10 scenery, 0x11/0x12 containers, 0x13/0x14 portals, 0x16/0x17 items in the
     vicinity (masks 8, 4, 2, 0x7fe0), all within 10 tiles here.
   - **Not built:** 0x31 / 0x54 areas (next PR); 0x38 toggle off (`flags` 2, no object flag
     changes are kept yet); 0x3a kill (nothing takes damage); 0x52 play sound (the id-to-file
     mapping 0x41a940 not decoded); the temporary record's header defaults (0x44c710).
   - **Real data:** 1875 of 2399 scripts run to an end (1731 before). Next: action 0x4d (82),
     0x50 (47), 0x31 (18), condition 0xb (18), action 0x38 (17), 0x52 (17).
5. **Gold and items:** done in 4c.
5. **Float text and messages** (0x40/0x41/0x72) as deltas to the right players.
6. Later: dialogue (4, 0x30, event 9), quest state, reputation, followers, timed events,
   teleports, combat-related actions, the remaining conditions.

## Open

- What decides the start line (B+4) besides 0. The theft checks for event 1. Attach point
  names (no table in the exe; 9 is dialogue).
- Script flags other than 4, 8 and 0x10. The 8 ignored header bytes.
