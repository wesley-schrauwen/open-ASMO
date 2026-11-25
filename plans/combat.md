# Combat: turn-based fights

Status: steps 1-2 built (`feat/combat-turns`, `feat/combat-npcs`); the rest planned.

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
- **Roll:** 1-100 + difficulty ≤ chance hits. Critical hit 0x4c81a0, critical miss 0x4c82e0
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
- **Critical multiplier** (0x4b5f40): × 3 at `chance + 10`, × 2 at `+ 30`, × 1.5 at `+ 60`, else
  × 1.5 (`chance` the weapon's `weapon_magic_crit_hit_effect`), on the hit point damage; other
  critical effects (knock down, stun, blind, cripple, item damage) by `weapon_crit_hit_chart` and
  the body part.
- **Applying** (0x4b4390): hit points `hp_damage` (field 0x1d); poison stat 0x18; fatigue. A
  critter's most hit points (0x43d5a0): `4 × hp_pts + hp_adj + WP + 2 × (level + ST) + 4`, as a
  PC's.
- **Death** (0x45d8d0: hit points < 1 → 0x45da20): script event 12 (dying); an NPC's killer
  gets a fifth of `npc_experience_worth`; each hit earned its share of the pool before
  (`npc_experience_pool`, damage percent × worth); the death art.

### NPCs (0x4a9650, 0x4ab460, 0x4aabe0)

- **Hostility** (0x4a9650): an attack makes the victim fight the attacker (its target in field
  0x11b, who hit it in 0x11c); allies within 2 / 8 / 15 tiles who notice it join (same faction,
  party); neutrals flee; a PC attacker's reaction drops (-10 at least).
- **States** (0x4aab00): idle, fighting (`npc_flags` 0x1), fleeing (`critter_flags` 0x10),
  surrendered (0x4000). Fleeing and surrender by `rules/ai_params.mes` (41 rows of 17 numbers,
  row by field 0x11a): flee at the row's HP percent (0), against the target's strength (1-3).
- **Targets** (0x4ab460): critters within `clamp(PE, 1, 10)` tiles it sees or hears, nearest
  first, hostile by: `npc_flags` 0x100 (kill on sight); a PC whose reaction is at or below the
  row's [10]; an alignment gap of the row's [12]; one fighting its friend.
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

## Steps

1. **Turns and the attack** (built, `feat/combat-turns`; `arcanum_sim::combat`).
   - `PlayerIntent::Attack` starts a fight (PCs within 15 tiles first, then fighting NPCs; the
     attacker leads) or strikes on the PC's turn, walking next to a melee target first as far
     as its points go; `PlayerIntent::EndTurn`. Moves in a fight: only on the PC's turn, 2 points
     a step, cut to what is left.
   - The attack as above without the light, range, cover, state and critical-effect terms;
     critical hits on a second roll at half the chance (builder's call: 0x4c81a0's luck term not
     read), the multiplier with no weapon bonus.
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
3. **The client:** the combat button, the AP bar and End Turn, clicking a critter to attack,
   the floats, the attack, hit and death animations, the battle cursor.
4. **Hooks:** script events 12-14 and hit / miss, action 0x24, dialogue `co`, PC death and the
   fate points that wait on attacks.
5. **Noticing:** kill-on-sight NPCs and monsters starting fights; the hover's to-hit panel.
