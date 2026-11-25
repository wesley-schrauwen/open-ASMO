# Checks by eye

Each item below is built and passes its tests, but nobody has looked at it in the game yet.
Tick an item once it has been seen working. Note anything wrong under it, with the PR it came
from. Each new PR adds its own items here. How to run the game for a check: `private/DEVELOPMENT.md` §6.

## Barter (#79, #88)

- [ ] Talking to a shopkeeper and choosing to trade opens the barter window: the PC's items on
      one side, the shop's on the other, and the shopkeeper's name shown.
- [ ] Hovering an item shows its price on the barter line. Buying takes the gold and moves the
      item over; selling does the reverse.
- [ ] Items the shop won't take, and buying without enough gold, show the refusal and change
      nothing.
- [ ] Closing and reopening the window shows the same stock. After 12-24 game hours (rest), the
      stock has been restocked.
- [ ] Leaving the map with the window open closes it.

## Vote popup (#81, #89)

Needs two players: a LAN host and a client (`DEBUG_ARCANUM_LISTEN` / `_JOIN`).

- [ ] When one player triggers a teleport, the other sees the black panel with the title
      "Confirm Group Action", then "<name> wants to go elsewhere. Go along?", then Yes and No.
- [ ] The text fits the panel; Yes and No turn cyan under the cursor.
- [ ] Both answer Yes: the party moves. Either answers No: nobody moves and the panel goes.
- [ ] The game clock keeps running and both players can act while the vote is open.
- [ ] A player who leaves during a vote drops out of it, and the vote still ends.
- [ ] Alone in the world, a teleport happens at once, with no panel.

## Teleports and map switches (#81, #82, #83)

- [ ] A script teleport within the map moves the party, and the camera follows.
- [ ] A teleport to another map (for example into Bessie Toone's mine, map 5001) clears the old
      map's sprites and shows the new map with its NPCs.
- [ ] Coming back shows the first map as it was left: changes are kept (moved and taken items,
      NPC inventories).
- [ ] Stepping on a jump point (`map.jmp`) starts the move, voted on as above.

## Dialogue (#78, #80, #86, #87)

- [ ] NPCs without a script greet by class or race and reaction, and bark when passed.
- [ ] Rumours, training and quest lines show their prices and take the gold.
- [ ] Directions (`D:` / `X:`) give a sensible direction and distance and mark the map.
