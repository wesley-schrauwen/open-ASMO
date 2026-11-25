# Companions

Status: following built (`feat/followers`, below); the player's control of followers is a plan.

The player manages their followers directly, the same way they manage their own character.

## What the player controls

- **Skill points:** how each follower's skill points are allocated.
- **Attributes:** where each follower's attribute points go.
- **Weapons:** what each follower wields.
- **Armour:** what each follower wears.
- **Items:** which items each follower may use and which they may not (for example, whether a
  follower may drink healing potions or use scrolls on their own).
- **No choosing their own items:** followers do not pick their own gear. The original equips a
  follower with the best of what it is given (probably 0x4654f0, called after inventory
  changes; not confirmed). Here that is
  switched off: a follower uses only what the player assigns. The original's behaviour is copied
  first, then switched off (owner's call).
- And so on: other per-follower choices to be added as the design grows.

## Following (built, `feat/followers`; `arcanum_sim::follow`)

Copied from the original first.

- **Joining** (`jo force line`, 0x41624d → 0x4ad950 / 0x45de90).
  - Refused for being too good (1) or too evil (2), for a reaction at or below the AI packet's D6
    (3), for following a PC at least as charismatic (4), for too many followers (5) or none
    allowed (6), or for being more than D9 levels above the PC (7).
  - A Master of Persuasion skips 1-4 and 7. `jo 1` skips the count, and that follower is not
    counted (`npc_flags` 0x400).
  - Refused: a `gd_npc` line by reason (1600, 1700, 1800, 2600-2900; override 12015 + ...) and the
    talk ends (builder's call). Joined: the talk goes on at NPC line `line`.
  - Max followers: Charisma / 4, plus one for an Expert or Master of Persuasion (stat 0xf).
- **Orders:**
  - `wa` waits (flag 0x8: keeps its leader, not counted, does not follow);
  - `uw force line` rejoins after the same check;
  - `sc` / `so` stay close / spread out (flag 0x10);
  - `lv` leaves.
  - Tests: `fo 1` follows this PC (Virgil's "I need you to do something..." has `fo 1, wa 0`), `wa`,
    `wt`.
- **Following** (0x4ac380 / 0x4346a0): farther than 4 tiles (7 spread out) the follower walks
  until within it. At 31 tiles or more it is put on the leader's tile (0x4ad4d0).
- **Moves:** a teleport within the map puts the followers on the leader's tile.
- **Map changes** (owner's rule, `feat/party-travel`): followers who are not waiting go with their
  leader, onto the same tile, as the original does (0x4d3760). Waiting ones stay where they are.
  - A follower away from its home map is a guest there. It is read from its home map's `.mob`
    files, added to the mobile objects on its tile, and tracked by its home
    (`Follower::away`: map, arrival tile, `npc_flags`). It is not spawned at home meanwhile.
  - Going back to its home map, it takes its home place again (it spawns when its home sector
    loads; the leash then brings it).
  - A guest that leaves (`lv`) goes home at the next map load (builder's call).
  - **Not built:**
    - a guest's other changes (inventory, met, attachments) are kept against its place on the
      map it is visiting, not its home;
    - a guest's tile is the arrival tile until the next move, as NPC positions are not saved.
- **Saved:** `WorldSave::followers` (map, NPC, leader player) and the NPC's flags in the sector's
  changes (`npc_flags`).
- **Multiplayer:** each follower follows the PC who recruited it.
- **Not built:**
  - **The wait timer:** 15 min to 30 days by field 0x12c, after which the follower leaves for
    good (0x4aa990).
  - **Per-tick re-checks:** the original re-checks each tick and the NPC may leave (0x4ac380).
    The warnings (`gd_npc` 5200-5400) are not built either.
  - **Hits:** the reaction lost when the leader hits a follower.
  - **Summoned and charmed** followers.
  - **Gold:** followers' gold in `$$` tests and payments (0x415335 / 0x415cc6).
  - **The follower menu** (inventory exchange, character sheet).
  - **The level catch-up** (0x4a6cb0).
  - **`pa`:** the party test.
  - **Where followers stand:** an NPC's position is not saved, so a follower stands at its home
    after loading until the leash brings it.
- **Data:** Virgil's AI packet is 5, but `Rules/gameai.mes` has only packet 100 ("generic AI"),
  which is used for any packet it lacks (builder's call: how the original maps packet numbers to
  rows was not traced). Virgil (level 1, alignment 1) passes the join check for a new character.

## New followers

### Mule

A literal mule that follows the party and provides extra inventory space.

## Related

- [Combat design](combat_design.md): the combat directives (target priority, role, stance,
  style) the player sets for companions, and the party stance chosen at the start of each turn
  cycle.
