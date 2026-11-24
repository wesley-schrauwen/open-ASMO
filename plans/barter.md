# Barter

Buying from and selling to NPCs: the inventory window in mode 1 (Arcanum.exe 0x572240). Findings
come from three Sonnet agents (2026-11-24); the price arithmetic, the reaction factor, the haggle
value and the line ranges were checked in the disassembly.

## Opening (owner's rules apply: host-run, private to the trader)

- A dialogue option `B:` (`gd_pc2` 300-399; 1600-1699 with a follower) sets state 3; its result
  is not run (0x413130). The talk ends and `0x572240(pc, npc, 1)` opens the window.
- **Goods** (0x45f650): the NPC's `npc_substitute_inventory` container when the NPC follows
  nobody and the container is within 20 tiles; else the NPC's own inventory. Opening moves the
  container's gold to the NPC (0x572640): the NPC holds the gold, the container the goods.
- The NPC says nothing on opening; closing does nothing more than close (0x573440).

## Prices (0x461f80)

- **Worth** (0x461f20): `item_worth`, at least 2, for a known item; 300 for one not known
  (0x464370: `item_magic_tech_complexity` above 0 and no `item_flags` 1). Gold: its quantity.
- **Haggle** (0x4c62e0, skill 9; jump table 0x4c63dc): level × 4 + 10, + 10 with base
  Intelligence 20 or more (0x4b0ee0), at most 95 (flag 0x800 at 0x5b6f64). The PC's is scaled by
  the game difficulty (easy +50 %, hard -25 %): no setting yet.
- **The NPC sells:** `p = worth + (100 - haggle) × multiplier × worth / 10000`
  (`npc_retail_price_multiplier`), then `p × f / 100` with the reaction factor `f` (0x4c1150:
  reaction clamped to 0-100; below 50 `(100 - r) × 2`, else `120 - 2r / 5`). At least
  `worth + 1`; 1 or 2 become 3.
- **The PC sells:** a multiplier `m` above 100 makes the worth `worth × (100 + (100 - m) × 3 / 8)
  / 100`; then `(haggle / 2 + 50) × worth / 100`, times hit points over the most; below 0 is 0,
  1 is 2. Reaction plays no part.
- **Refusals** (price 0): the NPC will not sell (0x462230) an `item_flags` 2 item or one it wears
  below Master haggle; it will not buy (0x462170) an item worth 0, a "proto type 6 with 0x400"
  item (0x5045a0), below Expert haggle one outside its trade kinds (0x464200, shop lists via
  0x4bf5d0), or stolen goods (`item_flags` 0x8000) unless it fences (`npc_flags` 0x4000).
- Data: `npc_retail_price_multiplier` is 0 for 2516 NPCs and 300 for 1210 (others 58-200).

## Trading

- Dragging an item across trades at once (drop handlers 0x575200 / 0x575360 → 0x5788c0, message
  0x51, done by 0x578ea0 on the host). There is no offer step. The box at (703, 236) (art 344-346,
  `Gamble_Box_*`) is gambling, not barter.
- Buying without the gold: `gd_cls` 2000 (0x413910), nothing moves. Selling to an NPC short of
  gold: it pays what it has (the price is capped, 0x575360).
- Reaction afterwards (0x4c11d0): over- or underpaying moves it; medium confidence, not built.

## Lines (0x413490-0x413a30 → 0x418780, `gd_npc`; override `base / 100 + 11999`)

| Case | Lines |
|---|---|
| The NPC sells, price | 800-899 (with haggle training, ` (N%)` over worth, at least 1) |
| Will not sell | 900-999 |
| Normally will not sell (a prefix) | 1000-1099 |
| The NPC buys, price | 1100-1199 |
| Will not buy | 1200-1299 |
| Normally will not buy (a prefix) | 1300-1399 |
| Buy for less (price, the NPC's gold) | 1400-1499 |
| Stolen goods | 4600-4699 |

## The window (0x5727b0, 0x576520)

- `Barter.art` (235) in place of `PDoll`; `Barter_Follower.ART` (825) for a follower.
- The NPC's portrait at (183, 33), its name centred at x 215, y 102 (200 × 20).
- The text line (89, 128, 253, 30): the line of the item under the cursor (0x578330).
- The NPC's grid from (8, 168): 10 columns, 7 rows shown (clip 224), scrolled by row.
- Paperdoll toggles (art 338/339): the PC's at (713, 37) / (751, 37), the NPC's at (8, 123) /
  (46, 123); the NPC's paperdoll `Barter_PD.ART` (337) at (0, 157), slot table 0x5cad78.

## Inventories and restocking

- NPCs on the map hold their stored list (`critter_inventory_list_idx`). The inventory source
  (`critter_inventory_source`) is rolled when an NPC is created (0x442260 → 0x463c60 →
  0x463e20), not when a map loads.
- **Restock** (0x4640c0): opening the window queues a timer of 12-24 hours (43.2-86.4 M ms). When
  it fires, the NPC's items are destroyed except those with item field 0x57 bit 0x2000 (its
  setter was not found), then the source is rolled on top; gold adds up.
- Data: 3792 NPCs; 1365 with a stored list, 374 with a source, 55 with a substitute container.

## Steps

1. **The host** (`arcanum_sim::barter`). Built.
   - Prices, refusals, haggle and reaction as above; `B:` opens barter (`BarterStarts`).
   - NPC inventories from the map's stored lists, kept per sector once traded (`npc_inventories`,
     saved); substitute containers and their gold.
   - `Buy` / `Sell` / `CloseBarter` inventory requests; each view (`LedgerDelta::Barter`) carries
     the goods, each item's price and line; refusals come as `BarterLine`.
   - **Not built:** trade kinds (every item passes); the 0x5045a0 refusal; the
     "normally will not" prefixes; the reaction change after a trade; item condition (items carry
     no damage) and per-item flags (identified, stolen: the prototype's are used); followers'
     free exchange.
2. **The window** (`inventory_screen.rs`). Built; not checked by eye.
   - Later (`feat/barter-restock`): the NPC's name at (215, 102); the window closes when the
     party goes to another map (and the loot window too).
   - `Barter.ART`, the NPC's grid at (8, 168) with 7 rows (gold hidden), scrolled by the wheel;
     dragging or right-clicking across buys or sells; the text line shows the hovered (or held)
     item's line, else the last refusal. Closing sends `CloseBarter`.
   - **Not built:** the NPC's portrait and name; the paperdoll toggles and the NPC's paperdoll
     (its worn items are not shown); the follower variant; the gamble box.
3. **Restocking** (built, `feat/barter-restock`): opening sets `SectorDelta::npc_restock` to
   12-24 game hours on; an opening after it purges the grid but for gold and the kinds the map
   list flags `item_flags` 0x2000 (104 placed items carry it; no prototype does; carried items
   keep no flags here, so it goes by kind), rolls `critter_inventory_source`, and wields the
   best. A substitute container rolls its own pending source once. Followers do not restock.
