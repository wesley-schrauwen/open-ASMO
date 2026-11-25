# Combat: turn-based fights

Status: steps 1-8 built (`feat/combat-turns`, `feat/combat-npcs`, `feat/combat-ui`, `feat/combat-hooks`, `feat/combat-notice`, `feat/combat-allies`, `feat/combat-surrender`, `feat/combat-criticals`).

The owner's rules (`plans/multiplayer.md` 3, `plans/combat_design.md`): combat is turn-based
only; the host is authoritative over every result; any client may start a fight; the party
members within the engagement radius join it, the others move freely in real time; a member who
walks in joins at the bottom of the order next round. The companion directives and party stance
of `combat_design.md` come after the original's rules work.

## Findings (Arcanum.exe)

### The encounter (combat module 0x4b3xxx-0x4b8xxx)

- **Combat mode** per critter: `critter_flags` 0x40000 (0x4b3d50 tests, 0x4b3f80 toggles; enter
  0x4b3f20, leave 0x4b3f50; leaving refused while attacked: `intgame.mes` 24). A PC entering it
  in turn-based mode starts the fight (0x4b71e0).
- **Start** (0x4b71e0): every critter within 15 tiles (`0x5b57b8`) of the PC, sorted with PCs
  and their followers first, the rest in the order found (0x4b7eb0: no initiative). Critters
  not fighting are skipped as their turns come (0x4b7580).
- **Rounds** (0x4b73a0): the list is gathered and sorted again each round; with no active
  critter the fight ends (0x4b7330). Joining mid-fight: 0x4b7e00 (appends, sorts again).
- **Turns** (0x4b6e70): action points `max(5, speed)` (stat 0xb), none carried over; the UI's
  AP bar shows them (0x5fc1ec). Next turn 0x4b7690; the player's end turn 0x4b7c90 (points to
  0, next turn); a round's end waits 1000 ms (0x4b7740).
- **Costs** (0x557370 dispatch):
  - a step 2 points walking, 1 running (0x4b7ba0: path cost × 2, × 1 running);
  - an attack by the weapon's speed (0x4b7c30): up to 20, `max(1, 8 − speed / 3)`; 21-24, 2;
    25 or more, 1. Speed (0x465cf0): bare hands 10; a weapon's `weapon_speed_factor` plus its
    magick part, + 5 when trained in its skill;
  - using an item or scenery 1 (+ steps), a door or container 2 (+ steps), others 4
    (0x4b7a20 / 0x4b79a0 / 0x4b7aa0);
  - a PC with points left may act past them (0x4b7790); its turn then ends.

### The attack (0x4b3170)

- **Skill** (0x465d80): none, melee; `weapon_flags` 0x40, throwing; tech, range ≥ 3 and
  ammunition 1-3 (bullets, charges, fuel), firearms (tech 1); ammunition 0 (arrows), bow; else
  melee.
- **Chance** (0x4c62e0 / 0x4c69f0): bow and melee 5 × level + 25, throwing 7 × level + 25,
  dodge 2 × level, firearms (tech) 6 × level; + 10 with effect 4; the difficulty setting scales
  a PC's; at most 95.
- **Difficulty** (0x4c8430): −(the weapon's to-hit and its magick part), + 5 a point of
  Strength short of `weapon_min_strength`, range (bow and throwing below Expert: + 5 a tile past
  PE / 2; out of range impossible), light (`(255 − light) × 30 / 255`), cover, the target's state
  (−30 / −50 helpless, −30 unnoticed), the attacker's (blind + 30, crippled arms + 20 / + 50).
- **Roll:** 1-100 + difficulty ≤ chance hits. Critical hit 0x4c81a0 (a new 1-100 roll on a
  hit, at most `chance / D`: `D` 20 for skills with flag 0x400 in table 0x5b6f64 (bow, dodge,
  melee, throwing, firearms), else 2, checked at 0x4c81e4; + the weapon's field 0x82 scaled by
  0x461590; backstab and effect terms), critical miss 0x4c82e0
  (`(100 − chance) / 2` and the weapon's wear, at least 2), its effects 0x4b6410 (drop, break,
  wild hit by `weapon_crit_miss_chart`).
- **Dodge:** a hit is rolled again by the defender with Dodge; dodged: "Dodge!"
  (`combat.mes` 11). A critical dodge may counter (table 0x5b57a8: 0, 10, 50, 100 by training).
- **No armour class** term was found in the roll.
- **Damage** (0x4b6680, 0x465f70): each type (normal, poison, electrical, fire, fatigue)
  between the weapon's bounds plus its magick part; melee adds the damage bonus (stat 9) to
  normal and fatigue; bare hands: a PC 1-5 normal and fatigue (`max(g − 25, 1)` to `g + 5`, `g`
  the gloves' `armor_unarmed_bonus_damage`), a critter its `npc_damage_idx` pairs.
- **Resistance** (0x4b6860): type by table 0x5b5804 (damage, poison, electrical, fire,
  damage); fatigue three quarters; damage less its percent.
- **Critical multiplier** (0x4b5f40): `X` the weapon's field 0x83 (`weapon_magic_crit_hit_effect`,
  not for ranged weapons) + 10 on a called shot; × 3 on a roll up to `X + 10`, else × 2 on a
  second up to `X + 30`, else × 1.5 (checked: + 0xa, + 0x14, + 0x1e steps); an NPC's on a PC
  × 1.5.
- **Critical effects** (0x4b5f40 sets bits, 0x4b4390 applies them; not for `critter_flags` 0x4):
  - body part (0x4b65a0, checked): roll ≤ 70 torso, ≤ 85 legs, ≤ 95 arms, else head;
  - `E = X + 5`, + 10 with `weapon_crit_hit_chart` (0x81) 1 or 3 (no weapon or ranged: 1), + 10 on
    a bare head; saves: a roll ≤ CO + mod (0x45f060);
  - stunned (`combat.mes` 16, `critter_flags` 0x20), unconscious (17, fatigue + 10-20),
    knocked down (18), blinded (4, 0x80; head, chart not 1), crippled arm (5, 0x100, both 0x200;
    drops the weapon 19 / shield 20), crippled leg (6, 0x400), scarred (3, PCs, effect 0x32),
    weapon damaged / dropped (8 / 19), armour damaged (7), helmet knocked off (20);
  - items worn there lose 10 hit points (field 0x1d); at 0 destroyed (23 / 24);
  - a Master's Heal (0x4c7160 case 10) clears 0x80-0x400.
- **Critical miss** (0x4b6410): half of them have effects, on the attacker: the weapon destroyed
  (22), a spell weapon's spell on the fumbler (9), then 0x4b5f40's effects on it.
- **Applying** (0x4b4390): hit points `hp_damage` (field 0x1d); poison stat 0x18; fatigue. A
  critter's most hit points (0x43d5a0): `4 × hp_pts + hp_adj + WP + 2 × (level + ST) + 4`, as a
  PC's.
- **Death** (0x45d8d0: hit points < 1 → 0x45da20): script event 12 (dying); an NPC's killer
  gets a fifth of `npc_experience_worth`; each hit earned its share of the pool before
  (`npc_experience_pool`, damage percent × worth); the death art.

### NPCs (0x4a9650, 0x4ab460, 0x4aabe0)

- **Hostility** (0x4a9650): an attack makes the victim fight the attacker (its target in field
  0x11b, who hit it in 0x11c; setter 0x4aabe0 state 1).
  - Followers of both sides (0x4a9b80 → 0x4a9c00) fight the other side; one that likes the
    victim stays out (0x4adcc0) and likes its leader 5 less.
  - Witnesses (0x4a9e10): NPCs within a square of the radius (table 0x5b50c0 `{2, 8, 15, 1}`; a
    hit passes index 2) round the victim, through 0x4a9f10. Sides by 0x4ae3a0: itself, its
    leader, the same side (0x45d110: the same top leader or an equal non-zero `npc_faction`,
    0x45d080; with a PC, leading it or the PC's reputation with its faction, 0x4c21e0). An ally
    of the victim who notices it fights the attacker; an ally of the attacker who notices it
    fights the victim; one of neither side, idle, not `OCF_NO_FLEE`, who notices the attacker
    flees (0x4aabe0 state 2).
  - Noticing: sight 0x4af260 (`clamp(PE, 1, 10)` tiles, cut by stealth) or hearing 0x4af470.
  - The victim's reaction to a PC attacker: lowered to column 10 if above it (−10 for an
    indirect hit, flag 1); a leader hitting its follower sets column 9 and may make it rebel
    (0x4ad950). Hit and death witnesses within 10 tiles like the attacker's leader 20 less
    (0x4aee50, not built).
- **The AI tick** (0x4a84f0): 0x4a92d0 picks the state and target (its table 0x4a94a4: idle
  looks for targets 0x4ab460; fighting keeps or drops its target; fleeing 0x4aaf50; surrendered
  takes heart at 80 % hit points or more one tick in 500, 0x4a945a), then 0x4ac180 acts on it
  (table 0x4ac238: idle 0x4ac380, fighting 0x4ac6e0, fleeing 0x4ac620, surrendered 0x4ac660)
  unless a move is under way (`+0x14`). A fleeing NPC with no move left surrenders (0x4ac620 →
  0x4aabe0 state 3); a surrendered one goes back to idle when its target is gone, dead or down
  (0x4ac660), and only its target's attack makes it decide again (0x4aa620 case 3).
- **Keeping away** (0x4ac6e0 → 0x4ac7b0): a fighter whose row has column 15 above 1 and is
  nearer than that, one tick in 20, walks back to that distance. The rows' comments agree:
  "Generic Bowman" 4, "close combat invis will move back 5 tiles" 5.
- **States** (0x4aab00 reads them, 0x4aabe0 sets them; checked: 0x4aab00 is the getter): idle,
  fighting (`npc_flags` 0x1), fleeing (`critter_flags` 0x10, its target in 0xe6),
  surrendered (0x4000, the same target). Setting fighting raises event 13, leaving it 14. Fleeing and surrender by `rules/ai_params.mes` (41 rows of 17 numbers,
  row by field 0x11a): flee at the row's HP percent (0), against the target's strength (1-3).
- **Targets** (0x4ab460): critters within `clamp(PE, 1, 10)` tiles it sees or hears, nearest
  first, hostile by (0x4ae120): `npc_flags` 0x100 (kill on sight) without 0x800; a PC whose
  reaction is at or below the row's column 10; an alignment gap of at least column 11 (checked:
  the agent read 12; most rows hold 2001 there); one fighting its friend.
- **Noticing** (0x4aa0d0, heartbeat): critters within 10 tiles it sees or hears.
- **Script events:** 13 entering a fight, 14 leaving it (0x4aabe0); 12 dying; action 0x24
  makes the object attack each of a set (no ally pull, no reaction).

### Presentation

- **Combat button:** bottom bar (86, 457), art 470 `Combat_Button.ART`, hover `intgame.mes`
  1002; release toggles combat mode (0x54ebf0).
- **AP bar:** art 294 `AP_Bar.ART` at (222, 412) over the bottom bar; End Turn button art 297 at
  (320, 9) in it; 20 slots at x 22, 35, 48, 61, 74, 93, … 287 (table 0x5caa48), y 10: art 295
  green for points left, 296 orange for what a planned move costs, 315 red past them.
- **Animations:** 5 the combat stance (and sneaking), 6 run, 7 knocked down, 18 the default
  death; 20 and 21 most likely the attacks (0x437990 times them by the weapon).
- **Floats** (`combat.mes`, colours white, red, green, blue, yellow at 0x5b8e6c): "HT 7" (line
  1, white) for damage; "Dodge!" (11); "Critical hit" (12); "Damage +50% / +100% / +200%"
  (13-15); fatigue "FT" (10, red), poison (0, green); red over PCs, yellow over the player's
  followers.
- **Cursor:** art 353 `battlecur.art` in combat mode; a hovered critter gets `hoverattack.art`
  (555). A click on a critter attacks (0x54ed30 → 0x4445a0).

### `ai_params.mes` columns (agent's reading of the row's stack offsets; 0, 10, 11 checked)

| Col | Use |
|---|---|
| 0 | flee at or below this hit point percent (0x4ab2f0) |
| 1, 2 | flee when the target's group is col 1 larger, or its summed level col 2 higher (0x4ab2f0, 0x4ae0a0) |
| 3 | do not flee from a target whose hit point percent is below it |
| 4 | a new target farther than this is ignored while fleeing (0x4ab030) |
| 5-8 | reaction, alignment gaps and level for 0x4ad950 / 0x4adb50 |
| 9 | the reaction a leader's hit sets on its follower |
| 10 | hostile at or below this reaction; a hit lowers the reaction to it |
| 11 | hostile at this alignment gap |
| 12 | alignment ceiling of liking (0x4adcc0) |
| 13 | idle behaviour roll (0x4abc70) |
| 14 | a percent chance in combat (0x4a8940) |
| 15 | keep-away distance of a fighter (0x4ac6e0 / 0x4ac7b0) |
| 16 | unknown (0x4aeb10) |

## Steps

1. **Turns and the attack** (built, `feat/combat-turns`; `arcanum_sim::combat`).
   - `PlayerIntent::Attack` starts a fight (PCs within 15 tiles first, then fighting NPCs; the
     attacker leads) or strikes on the PC's turn, walking next to a melee target first as far
     as its points go; `PlayerIntent::EndTurn`. Moves in a fight: only on the PC's turn, 2 points
     a step, cut to what is left.
   - The attack as above without the light, range, cover, state and critical-effect terms;
     critical hits on a second roll at half the chance (corrected in step 8: a twentieth), the
     multiplier with no weapon bonus.
   - NPCs: an attacked NPC fights back (`Fighting`), joins the fight and keeps out of its idle
     wandering; its turns pass for now. Damage kept as `SectorDelta::npc_damage`; at 0 hit
     points it dies as action 0x3a kills, the experience shared.
   - Resistances: worn armour's, summed (builder's call: 0x43d6d0 not read).
   - Deltas: `Combat { order, turn, ap }` (empty order: over), `Attacked { attacker, target,
     outcome }`.
2. **NPC turns** (built, `feat/combat-npcs`).
   - A fighting NPC's turn: flee at its row's hit point percent (column 0; `critter_flags`
     0x8000000 never), walking up to 8 tiles away (builder's call) and dropping out; else attack
     its target, or the nearest standing PC of the fight, walking next to it first, until out of
     points. NPCs no longer fighting, or dead, pass.
   - PCs take hits on their sheets (hit points, poison, fatigue); a PC with no hit points takes no
     turns. A fight with no NPC fighting or no PC standing ends at once.
   - `HostRng` is xorshift64* now: plain xorshift's neighbouring outputs correlated under `%`
     (a 25 % roll missed 46 times in a row).
   - Not built: allies joining, surrender, ranged NPCs keeping their distance, fleeing weighed
     against the target.
3. **The client** (built, `feat/combat-ui`; client `combat_ui`).
   - The combat button toggles combat mode; in it, or while the player's PC fights, a click on
     an NPC sends `PlayerIntent::Attack`.
   - The AP bar (art 294 at (222, 412)) while the PC fights: green lights (295) for its points
     on its turn, End Turn (297) sends `PlayerIntent::EndTurn`; the bar takes its clicks.
   - Floats from `combat.mes` for each attack: "HT n", "FT n", "Dodge!", "Critical hit",
     "Damage +x%", "Critical miss"; white over NPCs, red over PCs (`FloatColour::Red`).
   - Checked by screenshot (`DEBUG_ARCANUM_PANEL=fight`): the bar with two lights left after
     the walk to the half-ogre guard.
   - Not built: the orange and red lights of a planned move, the battle cursor and
     `hoverattack.art`, the attack / hit / death animations, the button's pressed look.
4. **Hooks** (built, `feat/combat-hooks`).
   - Action 0x24: each NPC of the set fights the object (0x447d2f; the original skips the ally
     pull and reaction there too). Dialogue result `co`: the NPC fights the PC and the talk ends.
     Both go through `CombatStarts`: a fight starts round the target if none runs.
   - Script events on the NPC, the PC the triggerer: 13 entering a fight (attacked or set on),
     14 leaving it (fleeing), 12 dying (after the kill: builder's call, the original runs it
     first and a script may stop the death).
   - A PC at 0 hit points lies down as an unconscious one does until healed (builder's call:
     the original's PC dies and the game ends; the owner decides what death is in co-op).
   - Real data: all 2399 scripts run to an end.
   - Not built: hit / miss events (7 / 8 not traced), the fate points that wait on attacks.
5. **Noticing** (built, `feat/combat-notice`).
   - Every 15 ticks (builder's call: the original on each heartbeat) each NPC not fighting,
     following, talking or dead looks for a PC within reach (`clamp(PE, 1, 10)` tiles, Perception
     cut by a prowler's value, or its hearing reach) it is hostile to (0x4ae120 as above); one
     found starts a fight (`CombatStarts`). Sight lines and facing are not counted.
   - The hover's to-hit panel in combat mode (0x554830): "To Hit:" and the weapon chance less the
     difficulty in the icon's place; the dodge and 0x461620 terms not counted.
   - Checked by screenshot: no fight starts at the house of the start tile `90330,84122` nor at
     the crash site.
6. **Others join** (built, `feat/combat-allies`; `combat::rally`).
   - Each attack (0x4a9650): NPCs within 15 tiles of the victim, not fighting yet: followers of
     either side, and NPCs allied to a side (`combat::allied`: a PC they follow, the same leader,
     the same non-zero `npc_faction`) who notice that side's critter (`notice_reach`), fight the
     other side and join the encounter (event 13); NPCs of neither side who notice the attacker,
     idle and not `OCF_NO_FLEE`, walk up to 8 tiles away (builder's call, as fleeing).
   - A PC attacker's victim likes it no more than its row's column 10.
   - The order: PCs, their fighting followers, the others (0x4b7eb0). Followers in a fight stop
     following and attack the nearest fighting NPC when their target is gone; a fight goes on
     while an NPC that follows no PC fights.
   - Distances are radii (owner's rule), not the original's square.
   - Not built: the bark, followers who like the victim staying out, a leader hitting its
     follower, the witnesses' −20 (0x4aee50), the PC's reputation with a faction, prowling
     attackers noticed less.
7. **Fleeing, surrender, keeping away** (built, `feat/combat-surrender`).
   - `should_flee` is 0x4ab2f0 in full: never from a target below column 3 percent; at or
     below column 0; else when the target's side is column 1 larger, or its levels column 2
     higher (a side: a PC and its followers, an NPC alone).
   - A fleeing NPC with nowhere to run (no path away within its points) surrenders
     (`combat::Surrendered`): out of the fight, still. It takes heart when whom it gave up to is
     gone, dead or down, or at 80 % hit points one tick in 500 (`recover_surrendered`); only
     that critter's attack makes it fight again. It notices no foes, does not wander and is not
     drawn into others' fights.
   - Keeping away: at the start of its turn, one turn in 20 (builder's call: the original rolls
     each AI tick), a fighter nearer than its column 15 distance walks back to it.
   - Attackers out of reach walk towards the target until in reach (ranged ones too).
   - Not built: the surrender bark and action 0x19, ammunition (not found in the attack), weapon
     switching, the one-shot items of 0x4ac910.
8. **Critical hits** (built, `feat/combat-criticals`).
   - The critical chance is a twentieth of the skill chance plus `weapon_magic_crit_hit_chance`
     (unscaled: 0x461590 not read); the multiplier by the weapon's critical effect in three rolls;
     an NPC's on a PC × 1.5.
   - Injuries by body part: blinded, one or both arms crippled, legs crippled
     (`combat::critical_injuries`), all with the roll `E` (builder's call: the original's rolls
     for each differ a little). Kept in the PC's `CharacterSheet::injuries` and NPCs'
     `critter_flags` (`SectorDelta::npc_critter_flags`). Blinded + 30 and crippled arms + 20 /
     + 50 to the attacker's difficulty (0x4c8430). A Master healer's Heal cures them (builder's
     call: the rank test not traced). Floats "Blinded", "Crippled arm", "Crippled leg".
   - Not built: stunning, knocking out and down (their mechanics not traced), scars, items
     dropped or damaged by criticals, crippled legs' effect, critical misses' effects.
