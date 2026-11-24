# Dialogue

Copying the original's dialogue: talking to NPCs, their lines, the PC's options, and what
choosing one does. Findings come from two Sonnet agents (2026-11-24) on Arcanum.exe; the
format and language are checked against all 709 `.dlg` files.

## Multiplayer (owner's rules)

- **Dialogue is private.** A conversation belongs to the player who started it. Only that
  player sees the options and chooses. It does not wait for the others; only big moves do
  (`plans/multiplayer.md`).
- **The host runs it** (the original keeps one dialogue state per player, 8 slots at
  0x66dab8, stride 0x1860, and clients only send their choice). Here: a `Conversation` per
  player on the host, and deltas to that player.
- **What others see:** the NPC's line and the PC's chosen line float over their heads in the
  original. Here every player near enough sees the floating lines (builder's call); only the
  talker gets the options.
- Results change shared state (global variables and flags, the NPC's flags, quest states,
  gold, items) through the host, as scripts do.

## Starting (0x42af00 → 0x57cc70)

- Clicking an NPC talks to it (the walk to it was not traced; use the door's range).
- **The NPC's dialogue script** (event 9) runs first (`0x441980(pc, npc, 0, 0, 9, 0)`). Script
  action 4 starts the conversation: the dialogue file is the running script's own (its
  number), and the start line is operand 0 (`0x567460`). The script then ends, skipping the
  default.
- **Without a script, or if it runs the default:** the NPC barks a generic line over its head
  (0x5681c0): lines 1900-1999 if its reaction is 4 or more, else 2000-2099, from the generic
  files. Not when it cannot talk (0x4ad800: dead, asleep, busy, fighting someone else).
- `0x567460` refuses when either party is dead or down, or the NPC cannot talk. Both turn to
  face each other and stop walking.
- **One line and no options** (state 4): the line just floats, and there is no conversation.

## The file (`arcanum_formats::dialog`, done)

- **Entries:** seven `{}` groups: number, text, female text or gender, intelligence, test or
  voice, response, result. Entries are sorted by number.
- **NPC line:** the intelligence group is empty. Its text is for a male PC, the female text
  for any other; the voice is the speech index; the result runs when it is shown.
- **PC option:**
  - Gender: empty for anyone, `1` male, anything else female.
  - Intelligence: `n > 0` needs at least `n`, `n < 0` at most `-n` (the "dumb" options). A
    PC whose Intelligence is above 4 counts as 1 when 0x45fc00 says so (Intelligence 4 or
    less, or effects 0x37, 0x4a, 0x4b).
  - Test and result: see the tables below.
  - Response: `n > 0` goes to line `n`, `0` ends, `n < 0` goes to line `-n` with state 2 (its
    consumer was not found; 2029 of 3032 point at PC options, probably "as if that option
    were chosen").
- **Options after an NPC line** are the PC options after it in number order, up to the next
  NPC line. They pass gender, intelligence and the test, and at most 5 are shown. If none
  pass, the PC says a random goodbye (generic 400-499) and the conversation ends.
- **Checked:** 709 files, 15287 NPC lines, 39472 PC options; opcode counts match the agent's
  tally. 12 tests and 11 results meet an unknown opcode: such a test fails, and such a result
  stops.

### Tests (0x4150d0; all must pass; `-n` flips to "at most")

- **PC stats and skills:**
  - `ps` persuasion; `ch` charisma; `pe` perception; `ha` haggle; `le` level;
  - `al`, `na` alignment;
  - `ma` / `ta` magick or tech aptitude;
  - `tr S L` training level; `sk S L` skill (S ≥ 12 is tech discipline S-12);
  - `sc C L` spell college.
- **Variables and flags:**
  - `gv N V` global variable; `gf N V` global flag;
  - `lf N V` / `lc N V` the NPC's own flag and counter (its dialogue attachment's);
  - `pv` / `pf` PC variable and flag.
- **Quests and story:** `qu N S` state equal; `qa` at least; `qb` at most; `ss` story state
  at least; `ru` / `rq` rumour known or quelled.
- **The NPC:** `re` reaction at least; `me 0/1` met before; `fo 0/1` follows the PC; `wa`
  its flag 0x118 bit 8.
- **Gold and items:** `$$ N` gold of the party at least; `in N` has item prototype N (`-N`:
  the NPC has it); `ni` has not; `pa N` a party member of prototype N.
- **Who and where:** `ra N` race + 1; `rp N` has reputation; `ar N` area known; `ia N` in
  area N.

### Results (0x415ba0; left to right; `co` last)

- **Gold and items:** `$$ N` gives gold (`-N` pays the NPC); `in N` gives the NPC item N (`-N`:
  the NPC gives it).
- **The NPC:** `re N` reaction (`+n`/`-n` add, `>n` at least, `<n` at most, bare sets); `lf`
  / `lc` its flag and counter; `co` it attacks; `nk` it dies.
- **Quests and story:** `qu Q S` quest state; `gv` / `gf` global; `pv` / `pf` PC; `ss` story
  state; `ru` / `rq` rumours; `rp N` reputation (`-N` removes); `mm N` an area on the map;
  `np` news.
- **The PC:** `al` alignment (as `re`); `xp N` experience tier; `fp` a fate point; `tr S L`
  training.
- **Followers:** `jo 0 V` join, then line V (or a refusal); `uw` a join variant; `lv` leave;
  `wa` wait; `sc` stay close; `so` spread out.
- **Flow and windows:** `fl N` shows line N as a closing line; `et S V` a training gate; `ce`
  the follower's sheet; `su` making items; `ii` identify; `ri` repair; `or` (not decoded).

### Text

- `@pcname@` and `@npcname@` are replaced with the names.
- An NPC line `G:` (optionally `G:i n, ...`) is a generic reaction line from the `gd_*`
  files. `I:` is a generic idle line (1000), after which options are shown.
- PC options with a prefix take a random generic line:
  - from `gd_pc2m` / `gd_pc2f` by the NPC's gender, or the `gd_dumb_*` pair for a dumb PC;
  - `Y:` 1-99, `N:` 100-199, `S:` 200-299, `B:` 300-399 barter, `E:` 400-499 goodbye, `T:`
    training, `H:` heal, `F:` 800-899, `K:` 1500-1599 more questions, `W:` 1800-1899 who are
    you, `P:` newspaper, `L:` world map;
  - `D:` / `X:` directions; `R:` news; `C:` story question; `A:` spell; `I:` insult; `Q:N` a
    quest line; `U:` training.
  - Any other option text is shown as written.
- **Voice:** `sound/speech/<dialogue number, 5 digits>/v<voice>_<m|f>.mp3` by the speaker's
  gender, falling back to `m`. None for voice 0.

## The window (0x4c9280, "TC")

- **Options panel:** no art. A black panel, 400x100, centred at `y = height - 181` (on
  800x600: x 200, y 419), widened to the widest option + 12. It holds 5 rows, 19 px apart
  from y 3, in Flare12. Colours: white, cyan under the cursor, blue when pressed. A click
  chooses. No numbers and no keys were found.
- **Lines:** the NPC's line floats over its head in white and stays until replaced (duration
  -2). The PC's chosen line floats over the PC in green. Floats are drawn in Flare12 (8 slots,
  0x4d5fe0).
- **Ending** (`0x5678d0`): the response 0, the NPC dying or being removed, a UI mode change.
  After the choice:
  - state 3 opens barter (`0x572240(pc, npc, 1)`);
  - 5 the follower's sheet;
  - 6 the world map;
  - 7 making items;
  - 8 identify or repair;
  - 9 `0x572240(…, 6)`.
- The game does not pause (low confidence).

## Steps

1. **The file and its language** (`arcanum_formats::dialog`) and this plan. Done.
2. **Conversations on the host** (`arcanum_sim::dialogue`). Done.
   - **Talking:** `PlayerIntent::Talk` walks to the NPC. Within use range the NPC's event 9
     script runs (`ScriptHost`). Its action 4 starts the conversation in the script's own
     dialogue file (`StartedDialogues`; any script may start one).
   - **Lines:** text by the PC's gender, names put in, the result run. The line floats for
     every player (`FloatText`, white, with the voice file; the PC's gender picks `_m`/`_f`,
     builder's call).
   - **Options:** only the talker gets them (`DialogueOptions`), and chooses with `Choose`.
     The chosen option floats green; its result runs, then its response. `-n` takes an NPC
     line as shown and a PC option as chosen (builder's reading).
   - **Ending:** `EndTalk`, response 0, or no options left (`DialogueEnded`). The PC has then
     met the NPC.
   - **Tests built:** `gv gf pv pf lf lc qu qa qb ss ru rq rp me $$ in ni ch pe le ra ps ha`.
   - **Results built:** `$$ gv gf pv pf lf lc qu ss ru rq rp fl`.
     - The NPC's `lf`/`lc` are its event 9 attachment's flags and counters, saved.
     - Quests, rumours, reputations and the NPCs met are saved with each PC; quelled rumours
       globally.
     - Gold changes send the inventory.
     - A test not built fails, and a result not built is skipped; both are reported
       (`DialogueReports`).
   - **Generic options:** `Y N S E F K W` take a random line from `gd_pc2m`/`gd_pc2f` (the
     `gd_dumb_` pair at Intelligence ≤ 4). Other prefixes are left out, and a `G:` line ends
     the conversation.
   - **Script runner additions:**
     - action 4;
     - condition 0x12 (leader; nobody follows anyone yet, so false);
     - flag tests on a set's objects (0xc-0x10, 0x1b, 0x1e, 0x25-0x27, 0x34).
   - **NPCs** carry `NpcSource` (sector, mobile index), so scripts can name them.
   - **Real data:**
     - Virgil (script 1324) at the crash site speaks his first line with its voice.
     - All his options test his reaction (`re62` / `re-61`). Reaction is not built, so they
       fail and the conversation ends there.
2b. **Reaction** (`arcanum_sim::reaction`). Done.
   - The reaction is the PC's stored value, else the NPC's `npc_reaction_base` (50), plus:
     - the PC's reaction modifier (from Beauty), unless the NPC ignores looks (`npc_flags`
       0x20000, `critter_flags` 0x2000800c);
     - the race table 0x5b684c (NPC race row, PC race column);
     - the PC's reputations' effects from `rules/gamerep.mes` by the NPC's origin and
       faction.
   - Changes (`re`: add, set, at least, at most) store the unmodified part, per PC and NPC,
     saved with the PC. Having a stored value is "met" (0x4c0c40). Levels follow 0x4c0fc0.
   - **Not built:**
     - the PC's effects on reaction;
     - charm;
     - the NPC's grudge list;
     - the original's ten-PC limit per NPC.
   - **Real data:** Virgil now offers his first two options. Choosing one shows his next line
     ("[The man's jaw drops open.]…") and its options.
3. **The client** (`dialogue_ui.rs`). Built; not yet checked by eye.
   - A click on an NPC walks to it and talks (`Talk` after the walk, as doors).
   - Lines float over the speaker's head: Flare12, centred, wrapped to 200 px; white for
     NPCs, green for the PC's choice. A new line from a speaker replaces the old.
     - The line of the NPC being talked to stays until the conversation moves on or ends.
     - Other lines last 3 s plus 1 s per 15 characters (builder's call).
   - The voice plays at the Voice volume (`_f`, falling back to `_m`); a new line stops the
     old one.
   - **Options panel:** black, (200, 419) 400x100, rows 19 px apart. White, cyan under the
     cursor, blue pressed; a click chooses.
   - Escape leaves the conversation. While talking, world clicks are blocked (builder's
     call).
   - `DEBUG_ARCANUM_TALK=1` talks to the nearest NPC once in the world.
   - **Not checked by eye:** the display was asleep and screenshots came out black.
4. **Generic lines** (`gd_*.mes`, the prefixes), barter (`B:`), and the other windows.
