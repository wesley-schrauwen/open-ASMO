# Companions

Status: plan, not implemented.

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

## New followers

### Mule

A literal mule that follows the party and provides extra inventory space.

## Related

- [Combat design](combat_design.md): the combat directives (target priority, role, stance,
  style) the player sets for companions, and the party stance chosen at the start of each turn
  cycle.
