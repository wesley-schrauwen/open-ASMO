# TODO

Things to pick up later. Larger plans live in `plans/`; project state and findings in `NOTES.md`.

## Art

- **Loading screen pocket watch:** the hand steps visibly because `LoadIndicator.ART` is 30
  pre-drawn frames (12° apart), and at one turn per 8 s each frame shows for ~0.27 s. The owner
  will make a replacement (GIF or animated SVG) with a smoothly rotating hand; the loading screen
  (`loading.rs`, `LoadingIndicator`) then needs to play it instead of the `.art` frames.

## World

- **Object blocking on the host:** `arcanum_sim::LoadedMap` has only the sectors' tile blocking
  masks. Walls, scenery and closed portals block too (their objects and prototype flags), but
  object blocking is not decoded yet, so the mask alone cannot decide where a PC can walk.
  Needed before movement validation is trusted (`plans/world_milestone.md`, step 4). Around
  the real start position the mask blocks none of the 41 x 41 tiles, so today the host accepts
  walking through everything there.
- **NPCs pause when a PC blocks them:** when NPC movement comes, an NPC whose next step is
  blocked by a PC stops for 5 s (150 ticks), then replans from where it stands; PCs replan at
  once (`plans/world_milestone.md`, "Pathfinding and movement validation"). Open: whether NPCs
  blocked by NPCs also pause, and what happens if the PC is still in the way after the pause
  (replan round it, or wait again). Also: an NPC sharing a tile with a PC walks to the nearest
  free tile once nothing on it is moving.
- **Sector streaming on the host:** sectors are loaded around each PC when it spawns
  (`SECTOR_RADIUS` = 1, a 3 x 3 block) and never unloaded. Load as PCs move, and unload what no
  PC is near.

## App

- **One copy of the game per machine.** A second copy would share the config and save folder and
  could overwrite the first one's files. Take an exclusive lock (a lock file in the config
  folder) at start and, if it is held, focus the running copy or say it is running and quit.
  Steam only half covers this: it refuses a second launch from the Steam library, but not one
  from the executable or a shortcut, and builds outside Steam get nothing. Local multiplayer
  testing needs two clients on one machine (Milestone A2), so the lock needs a development
  override (an env var, or a separate config folder per copy).

## Rules (after the faithful copy)

- **Training rank survives losing the skill:** the original never lowers a skill's training rank
  when its points drop (`NOTES.md` → Character rules → Copied oddities), so a Master can sit on a
  skill level below Master's 18. Likely left in because lowering a rank has wider effects: the
  rank's abilities, trainer and quest state, and refunds. Fix by dropping the rank to the highest
  one the new level still meets ([0,1,9,18]), and decide what happens to anything the rank paid
  for or unlocked.
