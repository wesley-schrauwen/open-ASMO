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
- **Without a script, or if it runs the default:** the NPC barks over its head (0x5681c0 →
  0x4132a0): the generic greeting of a `G:` line (0x419260), with the arguments
  `1 0, 2 0, ..., 8 0` (0x5a0a60) when the NPC follows the PC, else none. There is no
  conversation. Not when it cannot talk (0x4ad800). The earlier "1900-1999 / 2000-2099" reading
  was wrong: 1900-1999 is the PC's newspaper option.
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
  pass, the PC is offered one goodbye (`gd_pc2` 400-499, action 1: it ends; 0x414e60).
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
- An NPC line `G:` (optionally `G: k n, k fl n, ...`) is a generic greeting; `I:` an insult
  (`gd_rce` 1000). Both are followed by the line's options. See "Generic lines" below.
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

## Generic lines (0x418250-0x41a520; checked 2026-11-24)

- **Files** (table 0x5a063c, `mes\%s.mes`): `gd_pc2m/f` and `gd_dumb_pc2m/f` (the PC's lines, to
  a male or female NPC); `gd_cls_*` (class, 50 lines per `npc_social_class`, 11 classes noble to
  bandit), `gd_cls_pc2*`; `gd_rce_*` (50 per PC race, the first block for the NPC's own race);
  `gd_npc_*`; `gd_sto_*` (0x41a440, its own table). `m2f` is a male speaker to a female
  listener. The `dumb` files (0x45fc00: Intelligence 4 or less, or 0x4c2690 in 0x37/0x4a/0x4b)
  are for a dumb speaker: the NPC for its lines (0x418480, 0x4185f0), the PC for its own (0x4182d0,
  0x418390). NPC dumb files have one block per kind (no class or race offset); the PC's
  `gd_cls_dumb_pc2` keeps the class offset.
- **Picking** (0x4d4550, 0x4cef00): count the lines in `lo..=hi`, then take `lo + r`. A gap in a
  block makes some picks miss: `gd_cls_m2m` has no 8113, so 1 in 45 greetings of a wizard to a
  male PC at the best level hits it. The original then says "Error! Missing line 8113 in
  mes\gd_cls_m2m.mes" (0x4d43a0 formats 0x5b8cc4 when the lookup fails). Here the NPC says
  nothing (builder's call).
- **Overrides** (0x418870): first the NPC's override dialogue, the file of its script attachment
  31 (`scripts_idx` entry 0x1f), at a fixed line: class blocks `base / 1000 + 9999` (8000 →
  10007), race blocks `base / 1000 + 10999` (1000 → 11000), `gd_npc` blocks
  `base / 100 + 11999`, story `10000 + state`. 21 such files exist (`*Override*.dlg`, the
  followers). Only override lines carry a voice.
- **`G:` (0x419260):** arguments `k line`, `k fl line` or `k 0` for cases `k` 0-9: a line goes
  there (0x4197d0 → 0x414e60), `fl` closes with it (0x413a30, state 4), `0` turns the case off.
  The cases, in order:
  - 0 the NPC is dead (`gd_npc` 1500, speak with dead);
  - the NPC cannot talk (0x4af210): `gd_npc` 3000, and nothing more;
  - 1 the PC has a bad associate (0x45f460, `gd_cls` 18000);
  - 2 invisible (PC flags & 1, 22000); 3 body spell (& 0x3c, 19000); 4 polymorph (& 0x20000,
    20000); these four close the talk unless the NPC's class is 2;
  - 6 shrunk (& 0x80000, `gd_rce` 6000);
  - 7 underwear (armour type 0, 16000); 8 barbarian armour (type 7, 17000);
  - 5 is read by nobody; mirror image (21000) was not found;
  - 9 the greeting: one in five, a reputation's comment (0x4c1f80, 0x4c2100), else by meeting
    (0x4c0c40) and reaction level (0x4c0fc0), tables 0x4197a8 / 0x41978c:

    | Level | 0 | 1-2 | 3 | 4-5 | 6 |
    |---|---|---|---|---|---|
    | First meeting | cls 8000 | cls 9000 | cls 10000 | rce 2000 | rce 3000 |
    | Met before | cls 11000 | cls 12000 | cls 13000 | rce 4000 | rce 5000 |

  Then the options after the `G:` line.
- **Option prefixes** (0x416c10; letters at 0x5a0ab4: `z y x w u t s q p n l k h f e d c b a`,
  plus exact `I:` and `R:`):
  - `Y N S E F K W` `gd_pc2` blocks; `B` 300 (1600 with a follower), barter;
  - `A` thanks (`gd_cls_pc2` 1000); `I` insult (`gd_cls_pc2` 3000; 0x4191e0: reaction -10,
    below 21 the NPC insults back, `gd_rce` 1000, and the talk ends);
  - `C` the story question (`gd_pc2` 1700 + story state; 0x41a3e0: the NPC answers from
    `gd_sto`, its race + 100 × story state, and the PC gets one option, a thanks from
    `gd_cls_pc2` 1000, that goes on to the response);
  - `T:` training 500, `H:` healing 700, `D:`/`X:` directions and map marks 1300 / 1400, `R:`
    rumours (`gd_cls_pc2` 2000), `L:` the world map 2300, `P:` newspapers 1900, `Q:N` a quest
    line (0x4c4c00), `U:N` skill training 900, `Z:N` a spell 1000.
  - The result runs before the prefix's action, except for barter (0x413130).
- **Data** (702 of the 709 files; the 7 with spaces in their names were not dumped): 416 `G:`
  lines (24 with arguments, 14 with a `k` 9); 15 `I:` NPC lines and 122 `I:`
  options.
- **Not determined:** what class 2 stands for in the close check; 0x4c2690's 0x37/0x4a/0x4b;
  actions 4, 6, 7, 9 and 0x1a-0x1d.

## Services (0x414810 actions; checked 2026-11-24)

- **Hidden arguments:** an option's text after its prefix is kept after the shown line's NUL
  (0x416c10); lists are numbers or `lo-hi` ranges, comma separated, at most 100 (0x417f90).
- **The price dialogue** (0x418a40): `base` × the reaction factor (0x4c1150), at least 2. The NPC
  names it (`gd_cls` 1000, `%d`); yes (`gd_pc2` 1-99, action 4) pays, no (100-199) goes to the
  return. Too little gold: `gd_cls` 2000 and a "go on" (600-699) (0x418b30).
- **`R:$price, rumours`** (action 8, 0x418fc0; text `gd_cls_pc2` 2000 by class): the rumours
  the PC does not know (PC field 0x109) and nobody quelled (bits 0x6876c8); one at random. None:
  `gd_cls` 7000 and a "go on". Told (0x4190e0): `game_rd_npc_<npc>2<pc>` line
  `20 n + 1 + class`, else `20 n + 1` (line `20 n` is the logbook's); the PC knows it; one
  "thanks" (`gd_cls_pc2` 1000) to the return. Data: 239 `R:` options name 149 rumours; 28 have no
  text in any of the four files (the NPC says nothing; the PC still learns it).
- **`T:skills`** (action 5, 0x418ca0; text `gd_pc2` 500): `gd_cls` 3000, a skill name per skill
  (`mes/skill.mes`, 12-15 the tech skills), a "forget it" (800-899). Choosing one (0x418de0):
  trained already, `gd_cls` 4000; below the level for Apprentice (table 0x5b6f44:
  0 / 1 / 9 / 18), 5000; else the price, base 100. Paid (0x418f30): Apprentice (rank 1, the only
  rank given), `gd_cls` 6000 and `gd_pc2` 1000 to the return.
- **`Q:N`** (0x4c4c00, `Rules/gamequest.mes` row N, 23 numbers): the PC's quest state picks
  token 2-8 (reaction 20 or more), 9-15 (below 20) or 16-22 (Intelligence below 5); -1 leaves the
  option out, else that line of the file is the option.
- **`U:N`** (0x419a00 / 0x419b50): the NPC uses skill N (Heal 10, 14) on the PC for
  `(level + 2) × (rank + 1)`, reaction-scaled. **`H:`** (0x419830): healing spells 60-64 by the
  NPC's college-12 mastery, base price 100 (0x4b1740). **`D:` / `X:`** (0x419e70): directions to,
  and map marks of, unknown areas (`$price, areas`), `gd_npc` 100-799 by direction and distance.
  **`P:`** newspapers (0x41a520, 2 gold). **`L:`** state 6, the world map.

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
4. **Generic lines** (`arcanum_sim::dialogue`). Built.
   - `G:` lines: the greeting by class or race block, meeting and reaction level, and the `k` 9
     argument (a line, `fl` a closing line, `0` nothing). The options after the line follow.
   - `I:` NPC lines: the race insult.
   - Overrides from the NPC's attachment 31 dialogue, with their voices.
   - Options: `A:` thanks, `I:` insult (reaction -10, the NPC insults back below 21), `C:` the
     story question (the `gd_sto` answer, then one thanks to the response).
   - No options left: one goodbye that ends the talk.
   - Barks: an NPC whose dialogue script runs the default (or has none) says the greeting over
     its head (`TalkRequests::barks`); no conversation.
   - **Not built:**
     - the `G:` cases 0-8 (dead, cannot talk, bad associates, spells, armour);
     - reputation greetings;
     - the follower arguments for barks; can-talk (0x4ad800) and hostility (0x4ae120);
     - `B: T: H: D: X: R: P: L: Q: U: Z:` (left out and reported).
   - **Real data:**
     - 9856 greetings (every class, gender, dumb, PC race, level and meeting) find a line,
       except the `gd_cls_m2m` 8113 gap (the original shows an error line there; here, nothing).
     - Virgil still opens with his own line and offers both options.
   - The client needed no change: generic lines arrive as floats and options.
5. **Barter** (`B:`): see `plans/barter.md`.
6. **Services** (`arcanum_sim::dialogue`). Built: `R:` rumours, `T:` training, `Q:` quest
   lines, the price dialogue. Training sends the new sheet (`SheetUpdate`, ack 0) and saves it.
   - **Not built:** `U:`, `H:` (skill use and spells on the PC), `D:` / `X:` (areas), `P:`
     (newspapers), `L:` (no world map); what quells rumours; the rumour's timestamp and logbook.
   - **Real data:** every `Q:` names a row of `gamequest.mes`; `T:` lists name skills 0-15.
7. **Areas and directions** (`arcanum_sim::areas`). Built.
   - `mes/gamearea.mes` (82 areas: centre on the world map, name, description, radius) and
     `MapList.mes`'s `Area:` (73 maps). An object's area (0x4cb6a0): its map's, else on the world
     map the nearest centre within the area's radius (the original's region test 0x4be380 is not
     decoded); 0 none. Known areas are kept per PC (`PcScriptState::areas`).
   - Dialogue: tests `ar` (known) and `ia` (in it), result `mm` (mark). `D:` / `X:`
     (`$price, areas`, 0x419e70): left out when all are known (0x419e20); `gd_npc` 100 / 500,
     up to four unknown areas by name, "more" (`gd_pc2` 600) or "back" (800); the price
     dialogue when priced; then `gd_npc` 200/300/400 (`D:`) or 600/700 (`X:`, with the
     distance) + 10 × the direction, and `X:` marks the area; a "thanks" back.
   - The direction (0x4b8d50) and the distance (0x4b96f0, divided by 3168) were not read: here
     the screen angle of the line between the tiles, and the tile distance.
   - Scripts: action 0x31 marks an area known for PCs, 0x54 stores an object's area.
8. **The other tests and results** (built, `feat/dialogue-tests`; 0x4150d0 table 0x415b04,
   0x415ba0 table 0x41672c).
   - Every test opcode is built: `al` alignment (stat 0x13), `ma` / `ta` the aptitude up /
     down, `na` (alignment at least -a, or at most a when a < 0), `tr S L` training rank, `sk S
     L` skill level (12-15 the tech skills), `sc C L` college level (L ≤ 0: at most -L), `fo` /
     `pa` (nobody follows yet), `wa` / `wt` the NPC's `npc_flags` 0x8 / 0x20, and `in` / `ni`
     by the item's `name` (obj_f_name, as 2804 or 5222; -N: the NPC's or its substitute's).
     The earlier `in` test used basic prototype numbers: wrong.
   - Results: `in` moves an item by name (0x415fed: N > 0 PC to NPC, N < 0 NPC or its
     substitute container to PC; nothing made; no room: stays here, the original drops or
     destroys it); `al` set / add / at least / at most on the base alignment; `tr S L` one
     rank at a time with the level the rank needs; `xp N` adds `Rules/xp_quest.mes` line N
     (normal difficulty; no levelling yet); `fp` a fate point.
   - The sheet gains `alignment`, `experience` and `fate_points` (pre-mades read stats
     0x12-0x14); changes are applied after the step and sent (`SheetUpdate`).
   - **Not built:** `co` attack, `nk` kill, followers (`jo uw lv wa sc so`), `et`, the windows
     (`ce su ii ri`), `np` news, `or`.
   - **Not built:** finding areas by passing near them; the world map. Real data: the start
     tile is in area 2, the Crash Site; Bessie Toone's mine is area 1, Shrouded Hills.
