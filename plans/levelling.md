# Levelling

## Findings (Arcanum.exe)

- **The table:** `Rules/xp_level.mes`, read by 0x4a6600 into 0x5f5c94: entry i is line i + 1, and
  entry 0 is forced to 0. Entry L is the experience needed to leave level L (line 2: 2100).
  Line 51 (1,380,000) is read even though the file says only 1-50 are valid.
- **Experience to the next level** (0x4a68b0): `entry[level] - experience`, or 999999 from level
  51. The character screen's "XP Next" shows it (0x55b510).
- **Gaining experience** (0x45f110):
  - PCs only; nothing for 0.
  - Easy difficulty gives +50% and hard -25%.
  - In multiplayer the amount is split among the team.
  - `xpgain` effects apply (0x4eabc0, the effect table's "other" entry 9).
  - Then stat 0x12 is set.
- **Level-up** (0x4a69c0, from the stat setter 0x4b0980 case 0x12):
  - While the level is below 51 (stat maximum, 0x5b5204) and nothing more is needed, the level
    rises by one.
  - Each level gives one character point, two at every fifth (0x4a6980).
  - Reaching 51 instead sets the experience back to line 50's (1,300,000), so 50 is the top.
  - After the loop:
    - followers are raised to the PC's level (0x4a6cb0);
    - an auto-level scheme may spend the points (0x4a6d40 / 0x4a7030, `mes/level.mes`);
    - the UI is told (0x460630 and the rest).
- **Fate points** come only from dialogue (`fp`), never from levels. Hit points and fatigue
  follow from the level through their formulas.

## Built (`feat/levelling`)

- **Rules:**
  - `RulesData::xp_levels`, `xp_to_next`, `gain_experience`, `level_points`;
  - `xpgain` effects apply.
- **Dialogue `xp` results:**
  - they go through `gain_experience`;
  - each level reached sends "Congratulations! You are now level %d." (`mes/intgame.mes` 11) to
    the player (builder's call: the original's UI callback was not traced to that line);
  - the PC's status goes to everyone.
- **Shared experience** (owner's rule, `feat/party-travel`): dialogue experience is split equally
  among every PC in the world, at least 1 each, as the original splits it among the living
  members of the team in multiplayer (0x45f110 counts them via 0x440fc0 / 0x4ba020 and
  divides). Each share goes through that PC's `xpgain` effects and level-ups
  (`ExperienceAwards` → `share_experience`).
- **The character screen** shows the level and "XP Next" from the sheet. Both were fixed values
  before.

## Not built

- Difficulty: here always normal.
- Followers catching up, auto-level schemes.
- Alignment and fate points on the character screen. The original's gauge and its place were
  not traced.
- Experience from kills (`Rules/xp_critter.mes`) and from scripts.

## Real data

- The table reads 51 entries: 0, 2100, ..., 1,300,000 at entry 49.
- 5000 experience takes a new character to level 3 with two points.
