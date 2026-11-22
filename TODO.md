# TODO

Things to pick up later. Larger plans live in `plans/`; project state and findings in `NOTES.md`.

## Art

- **Loading screen pocket watch:** the hand steps visibly because `LoadIndicator.ART` is 30
  pre-drawn frames (12° apart), and at one turn per 8 s each frame shows for ~0.27 s. The owner
  will make a replacement (GIF or animated SVG) with a smoothly rotating hand; the loading screen
  (`loading.rs`, `LoadingIndicator`) then needs to play it instead of the `.art` frames.

## World

- **Blocking leftovers** (`arcanum_sim::blocking`, `doors`): sinking and icy tiles (flags `s`,
  `i`); paths through locked doors for a PC with the key (0x4aeb70 in the pathing, 0x43fe00:
  today every locked door blocks paths, though the key opens it);
  NPCs opening doors, and NPCs that cannot (pathflag 0x2).
- **NPCs pause when a PC blocks them:** when NPC movement comes, an NPC whose next step is
  blocked by a PC stops for 5 s (150 ticks), then replans from where it stands; PCs replan at
  once (`plans/world_milestone.md`, "Pathfinding and movement validation"). Open: whether NPCs
  blocked by NPCs also pause, and what happens if the PC is still in the way after the pause
  (replan round it, or wait again). Also: an NPC sharing a tile with a PC walks to the nearest
  free tile once nothing on it is moving.
- **World drawing gaps** (`world_render`, `NOTES.md` → "Objects on screen"):
  - wall cutaway near the PC;
  - roofs (pass 0x439140);
  - shadows, overlays and blit effects (`blit_flags`, `blit_scale`, flag 0x80);
- **Streaming leftovers:** NPC state is lost when their sector unloads (nothing changes it yet);
  the client loads only around its own PC, so other players' PCs far away walk over nothing
  drawn (they are off screen anyway).

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
