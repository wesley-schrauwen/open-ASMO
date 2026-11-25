//! Barter: buying from and selling to NPCs (Arcanum.exe: the inventory window in mode 1,
//! 0x572240; prices 0x461f80). See `plans/barter.md`.
//!
//! **Prices** ([`price`], 0x461f80; integer arithmetic throughout):
//! - **Worth** ([`worth`], 0x461f20): `item_worth`, at least 2, for a known item; 300 for one not
//!   known (0x464370: an item of `item_magic_tech_complexity` above 0 without `item_flags` 1,
//!   identified). Gold is worth its quantity.
//! - **The NPC sells** to the PC: `worth + (100 - haggle) × multiplier × worth / 10000`
//!   (`npc_retail_price_multiplier`), then × the reaction factor / 100 ([`reaction_factor`],
//!   0x4c1150); at least `worth + 1`, and never 1 or 2 (3).
//! - **The PC sells** to the NPC: a multiplier above 100 lowers the worth to
//!   `worth × (100 + (100 - m) × 3 / 8) / 100`; then `(haggle / 2 + 50) × worth / 100`, scaled by
//!   the item's condition (hit points over its most); never below 0, and 1 becomes 2.
//! - **Haggle** ([`haggle`], 0x4c62e0 skill 9): the skill's level × 4 + 10, + 10 with a base
//!   Intelligence of 20 or more, capped at 95. The original scales a PC's by the game difficulty
//!   (easy +50 %, hard -25 %); there is no difficulty setting, so it is unscaled.
//! - **Refusals:** the NPC will not sell ([`will_sell`], 0x462230) an item flagged `item_flags` 2
//!   or one it wears, unless the PC is a Master of haggle; it will not buy ([`will_buy`],
//!   0x462170) an item worth 0, a stolen one (`item_flags` 0x8000) unless it fences
//!   (`npc_flags` 0x4000), or, below Expert haggle, one outside the kinds it trades in. A refused
//!   item's price is 0.
//!
//! **Restocking** (0x4640c0 → 0x463c60 → 0x463e20): opening the window sets the NPC's restock
//! for 12 to 24 game hours on (`SectorDelta::npc_restock`; each opening moves it). When it has
//! come by the next opening, the NPC's items go but for what it wears, its gold and the kinds
//! its map list flags `item_flags` 0x2000 (the original keeps 0x2000-flagged items; the flag's
//! setter was not found, and carried items keep no flags of their own here), and its inventory
//! source rolls again (`critter_inventory_source`, `Rules/InvenSource.mes`); then it wields its
//! best. NPCs following someone do not restock. A substitute container rolls its own source
//! once, when it is still pending (`container_flags` 0x200), as the loot window's do.
//!
//! Not built: the kinds an NPC trades in (0x464200 with the shop lists of 0x4bf5d0: every item
//! passes); the "proto type 6 with 0x400" refusal (0x5045a0, not decoded); item condition (items
//! carry no damage: always whole).

use std::collections::BTreeMap;

use arcanum_formats::map::SectorId;
use arcanum_formats::object::{TYPE_CONTAINER, TYPE_GOLD, Value};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{CarriedItem, GroundItemId, Inventory, ItemInfo, LedgerDelta, LoadedMap, NetId, Outbox, PlayerControlled, PlayerId, ScriptObject, Sheet, SimRules, WorldState};

/// The haggle skill (`mes/skill.mes` order).
pub const SKILL_HAGGLE: usize = 9;
/// The highest haggle value (skill flag 0x800, table 0x5b6f64).
pub const MAX_HAGGLE: i32 = 95;
/// The worth of an item not known (0x461f20).
pub const UNKNOWN_WORTH: i32 = 300;
/// `item_flags`: identified.
pub const ITEM_IDENTIFIED: i32 = 0x1;
/// `item_flags` 2: an item the NPC will only sell to a Master of haggle (name not found).
pub const ITEM_KEPT: i32 = 0x2;
/// `item_flags`: stolen.
pub const ITEM_STOLEN: i32 = 0x8000;
/// `npc_flags`: the NPC buys stolen goods.
pub const NPC_FENCE: i32 = 0x4000;
/// Haggle training ranks (0x4c60c0).
pub const EXPERT: u8 = 2;
pub const MASTER: u8 = 3;

/// A critter's haggle value (0x4c62e0): skill level 0-20, base Intelligence.
pub fn haggle(level: i32, intelligence: i32) -> i32 {
    let bonus = if intelligence >= 20 { 10 } else { 0 };
    (level * 4 + 10 + bonus).clamp(0, MAX_HAGGLE)
}

/// An item's worth (0x461f20, 0x464370), for `quantity` of it.
pub fn worth(info: &ItemInfo, item_flags: i32, quantity: i32) -> i32 {
    if info.kind == TYPE_GOLD {
        return quantity;
    }
    let known = info.complexity <= 0 || item_flags & ITEM_IDENTIFIED != 0;
    if known { info.worth.max(2) } else { UNKNOWN_WORTH }
}

/// The factor in percent the NPC's reaction puts on its prices (0x4c1150).
pub fn reaction_factor(reaction: i32) -> i32 {
    let r = reaction.clamp(0, 100);
    if r < 50 { (100 - r) * 2 } else { 120 - 2 * r / 5 }
}

/// What the NPC charges for an item of `worth`: its `multiplier`, the PC's `haggle` and the NPC's
/// `reaction` to the PC.
pub fn buy_price(worth: i32, haggle: i32, multiplier: i32, reaction: i32) -> i32 {
    let p = worth + (100 - haggle) * multiplier * worth / 10000;
    let p = p * reaction_factor(reaction) / 100;
    let p = if p <= worth { worth + 1 } else { p };
    if p == 1 || p == 2 { 3 } else { p }
}

/// What the NPC pays for an item of `worth` in condition `hp` of `max_hp`.
pub fn sell_price(worth: i32, haggle: i32, multiplier: i32, hp: i32, max_hp: i32) -> i32 {
    let worth = if multiplier > 100 { worth * (100 + (100 - multiplier) * 3 / 8) / 100 } else { worth };
    let mut p = (haggle / 2 + 50) * worth / 100;
    if hp < max_hp && max_hp > 0 {
        p = hp * p / max_hp;
    }
    match p.max(0) {
        1 => 2,
        p => p,
    }
}

/// Whether the NPC sells the item (0x462230): flagged or worn ones only to a Master.
pub fn will_sell(haggle_training: u8, item_flags: i32, worn: bool) -> bool {
    haggle_training >= MASTER || (item_flags & ITEM_KEPT == 0 && !worn)
}

/// Whether the NPC buys the item (0x462170). `trades_in`: the item is of a kind the NPC trades
/// in (not built: always true).
pub fn will_buy(worth: i32, haggle_training: u8, trades_in: bool, item_flags: i32, npc_flags: i32) -> bool {
    if worth == 0 {
        return false;
    }
    if haggle_training < EXPERT && !trades_in {
        return false;
    }
    item_flags & ITEM_STOLEN == 0 || npc_flags & NPC_FENCE != 0
}

/// Who sells in a trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seller {
    Npc,
    Pc,
}

/// What an NPC needs for prices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Trader {
    /// `npc_retail_price_multiplier`.
    pub multiplier: i32,
    /// `npc_flags`.
    pub flags: i32,
    /// The NPC's reaction to the PC.
    pub reaction: i32,
}

/// The price of `quantity` of an item (0x461f80); 0 when refused. `haggle` and `training` are the
/// PC's.
#[allow(clippy::too_many_arguments)]
pub fn price(info: &ItemInfo, item_flags: i32, quantity: i32, worn: bool, seller: Seller, trader: Trader, haggle: i32, training: u8) -> i32 {
    let base = worth(info, item_flags, quantity);
    if info.kind == TYPE_GOLD {
        return base;
    }
    match seller {
        Seller::Npc if will_sell(training, info.item_flags | item_flags, worn) => buy_price(base, haggle, trader.multiplier, trader.reaction),
        Seller::Pc if will_buy(info.worth, training, true, info.item_flags | item_flags, trader.flags) => sell_price(base, haggle, trader.multiplier, 1, 1),
        _ => 0,
    }
}

/// The substitute inventory's greatest distance from its NPC, in tiles (0x45f650).
pub const SUBSTITUTE_RANGE: u32 = 20;

/// Where the barter NPC's goods are (0x45f650): its own inventory, or its substitute container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stock {
    /// The NPC itself (its place in the sector's mobile objects).
    Npc { sector: SectorId, index: u32 },
    /// The container named by `npc_substitute_inventory`.
    Container(GroundItemId),
}

/// A PC's open barter window: with which NPC, and where its goods are.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarterOpen {
    pub npc: NetId,
    pub sector: SectorId,
    pub index: u32,
    pub stock: Stock,
    /// A pick-pocket window ([`crate::pickpocket`]): taking is stealing.
    pub pickpocket: bool,
}

/// Barter windows to open: the player, the NPC and its map object.
#[derive(Resource, Debug, Default)]
pub struct BarterStarts(pub Vec<(PlayerId, NetId, ScriptObject)>);

/// Barter requests taken from the inventory requests (player, request number, request).
#[derive(Resource, Debug, Default)]
pub struct BarterRequests(pub Vec<(PlayerId, u32, crate::InventoryRequest)>);

/// What a player's barter window shows: the NPC's goods, what each costs and what the NPC pays
/// for each of the PC's items (0 refused), and the NPC's gold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BarterView {
    pub npc: NetId,
    pub npc_male: bool,
    /// The NPC's name (`mes/description.mes`), shown at (215, 102).
    pub npc_name: String,
    pub stock: Inventory,
    /// The NPC's goods' prices, by location.
    pub buy: BTreeMap<i32, Offer>,
    /// What the NPC pays for the PC's items, by location.
    pub sell: BTreeMap<i32, Offer>,
    pub npc_gold: i32,
    /// A pick-pocket window ([`crate::pickpocket`]): no prices; taking is stealing.
    #[serde(default)]
    pub pickpocket: bool,
}

/// An item's price and the line the window shows for it under the cursor (0x578330).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub price: i32,
    pub line: String,
}

impl LoadedMap {
    /// The goods of `stock`.
    pub fn stock(&self, stock: Stock) -> Option<Inventory> {
        match stock {
            Stock::Npc { sector, index } => self.npc_inventory(sector, index),
            Stock::Container(id) => self.pile(id).map(|p| p.inventory),
        }
    }

    fn set_stock(&mut self, stock: Stock, inventory: Inventory) {
        match stock {
            Stock::Npc { sector, index } => self.set_npc_inventory(sector, index, inventory),
            Stock::Container(id) => {
                if let Some(mut pile) = self.pile(id) {
                    pile.inventory = inventory;
                    self.set_pile(id, Some(pile));
                }
            }
        }
    }

    /// NPC `index`'s substitute inventory (0x45f650): `npc_substitute_inventory`, when the NPC
    /// follows nobody and the container is within [`SUBSTITUTE_RANGE`].
    pub fn substitute(&self, sector: SectorId, index: u32) -> Option<GroundItemId> {
        let npc = self.mobiles.get(&sector)?.get(index as usize)?;
        if matches!(npc.field("npc_leader"), Some(Value::ObjectRef(Some(_)))) {
            return None;
        }
        let Some(Value::ObjectRef(Some(id))) = npc.field("npc_substitute_inventory") else { return None };
        let at = crate::object_location(npc)?;
        self.mobiles.iter().find_map(|(&s, objects)| {
            let i = objects.iter().position(|o| o.id == *id && o.object_type == TYPE_CONTAINER)?;
            let there = crate::object_location(&objects[i])?;
            crate::within_radius(at, there, SUBSTITUTE_RANGE).then_some(GroundItemId::Container { sector: s, index: i as u32 })
        })
    }
}

/// What prices need for one PC trading with one NPC.
struct Deal<'a> {
    info: &'a dyn Fn(u32) -> Option<ItemInfo>,
    trader: Trader,
    haggle: i32,
    training: u8,
}

impl Deal<'_> {
    fn price(&self, item: &CarriedItem, seller: Seller) -> Option<i32> {
        let info = (self.info)(item.prototype)?;
        let worn = seller == Seller::Npc && (crate::FIRST_SLOT..=crate::LAST_SLOT).contains(&item.location);
        (info.kind != TYPE_GOLD).then(|| price(&info, 0, item.quantity, worn, seller, self.trader, self.haggle, self.training))
    }

    fn worth(&self, item: &CarriedItem) -> i32 {
        (self.info)(item.prototype).map_or(0, |info| worth(&info, 0, item.quantity))
    }
}

/// Whether an item at `location` is in the grid (not worn, not on the hotkeys).
fn goods_grid(location: i32) -> bool {
    !(crate::FIRST_SLOT..=crate::LAST_SLOT).contains(&location)
}

/// Fills `%d`s in order.
pub fn with_numbers(text: &str, numbers: &[i32]) -> String {
    let mut out = String::new();
    let mut numbers = numbers.iter();
    let mut parts = text.split("%d");
    out.push_str(parts.next().unwrap_or(""));
    for part in parts {
        out.push_str(&numbers.next().map_or_else(String::new, i32::to_string));
        out.push_str(part);
    }
    out
}

type TraderQuery<'a> = (Entity, &'a NetId, &'a PlayerControlled, &'a Sheet, &'a mut Inventory, Option<&'a BarterOpen>);

/// The world the barter systems work on.
#[derive(bevy_ecs::system::SystemParam)]
pub struct BarterWorld<'w> {
    map: ResMut<'w, LoadedMap>,
    world: ResMut<'w, WorldState>,
    outbox: ResMut<'w, Outbox>,
    rules: Res<'w, SimRules>,
    vfs: Option<Res<'w, crate::SimVfs>>,
    reputations: ResMut<'w, crate::ReputationTable>,
    generic: ResMut<'w, crate::GenericLines>,
    rng: ResMut<'w, crate::HostRng>,
    clock: Res<'w, crate::GameClock>,
    caught: ResMut<'w, crate::pickpocket::CaughtThieves>,
}

/// The restock delay's bounds, in game milliseconds (0x4640c0: 43,200,000 to 86,400,000).
pub const RESTOCK_MS: (i32, i32) = (43_200_000, 86_400_000);

impl BarterWorld<'_> {
    /// Rolls the goods that are due (see the module docs) and sets the next restock.
    fn restock(&mut self, sector: SectorId, index: u32, stock: Stock) {
        let vfs = self.vfs.as_ref().map(|v| v.0.clone());
        let vfs_guard = vfs.as_ref().and_then(|v| v.read().ok());
        let vfs = vfs_guard.as_deref();
        let (prototypes, art) = (self.map.prototypes.clone(), self.map.art.clone());
        let info = crate::item_info(&prototypes, |id| crate::world::art_size(vfs, art.as_deref(), id));
        let mut rng = crate::Rng(self.rng.next_u64() as u32);
        let now = self.clock.now;
        match stock {
            Stock::Container(id) => {
                let GroundItemId::Container { sector: cs, index: ci } = id else { return };
                let rolled = self.map.changes.get(&cs).is_some_and(|c| c.containers.contains_key(&ci));
                let Some(object) = self.map.container_object(id) else { return };
                let pending = self.map.prototypes.int(object, "container_flags").unwrap_or(0) & crate::OCOF_INVEN_SPAWN_PENDING != 0;
                let source = self.map.prototypes.int(object, "container_inventory_source").and_then(|s| u32::try_from(s).ok()).filter(|&s| s != 0);
                if rolled || !pending {
                    return;
                }
                let (Some(source), Some(mut pile)) = (source.and_then(|s| crate::inventory_source(vfs?, s)), self.map.pile(id)) else { return };
                crate::roll_source(&mut pile.inventory, &source, &mut rng, &info);
                self.map.set_pile(id, Some(pile));
                crate::ground::save_change(&mut self.world, &self.map, cs);
            }
            Stock::Npc { .. } => {
                let Some(npc) = self.map.mobiles.get(&sector).and_then(|m| m.get(index as usize)) else { return };
                if matches!(npc.field("npc_leader"), Some(Value::ObjectRef(Some(_)))) {
                    return;
                }
                let Some(source) = self.map.prototypes.int(npc, "critter_inventory_source").and_then(|s| u32::try_from(s).ok()).filter(|&s| s != 0) else { return };
                let kept = self.map.kept_kinds(npc);
                let due = self.map.changes.get(&sector).and_then(|c| c.npc_restock.get(&index)).copied();
                if due.is_some_and(|d| d <= now)
                    && let Some(source) = vfs.and_then(|v| crate::inventory_source(v, source))
                    && let Some(mut goods) = self.map.npc_inventory(sector, index)
                {
                    goods.items.retain(|i| !goods_grid(i.location) || i.prototype == crate::GOLD || kept.contains(&i.prototype));
                    crate::roll_source(&mut goods, &source, &mut rng, &info);
                    goods.wield_best(&info);
                    self.map.set_npc_inventory(sector, index, goods);
                }
                let delay = self.rng.range(RESTOCK_MS.0, RESTOCK_MS.1) as u64;
                self.map.changes.entry(sector).or_default().npc_restock.insert(index, now.plus_ms(delay));
                crate::ground::save_change(&mut self.world, &self.map, sector);
            }
        }
    }

    /// The deal for `player`'s PC with NPC `index` of `sector`, and the NPC's details.
    fn deal_parts(&mut self, player: PlayerId, sheet: &arcanum_rules::CharacterSheet, sector: SectorId, index: u32) -> Option<(Trader, i32, u8, crate::NpcInfo)> {
        let vfs_guard = self.vfs.as_ref().and_then(|v| v.0.read().ok());
        let vfs = vfs_guard.as_deref();
        let reputations = &*self.reputations.0.get_or_insert_with(|| {
            vfs.and_then(|v| v.read(crate::GAME_REP).ok()).and_then(|d| arcanum_formats::mes::MesFile::parse(&d).ok()).map(|m| crate::ReputationEffects::parse(&m)).unwrap_or_default()
        });
        let object = ScriptObject::Mobile { sector, index };
        let info = crate::npc_details(&self.map, object, &mut self.generic, vfs);
        let npc = self.map.mobiles.get(&sector)?.get(index as usize)?;
        let pc_state = self.world.0.as_ref()?.players.get(&player).map(|p| p.script_state.clone()).unwrap_or_default();
        let reaction = crate::reaction_to(&pc_state, &crate::met_key(object), &info.reaction, &self.rules.0, sheet, reputations);
        let trader = Trader {
            multiplier: self.map.prototypes.int(npc, "npc_retail_price_multiplier").unwrap_or(0),
            flags: self.map.prototypes.int(npc, "npc_flags").unwrap_or(0),
            reaction,
        };
        let haggle = haggle(self.rules.0.skill_level(sheet, SKILL_HAGGLE), sheet.base[4]);
        Some((trader, haggle, sheet.training[SKILL_HAGGLE], info))
    }

    /// A generic NPC line for `pc` (the override first), with `%d`s filled.
    fn line(&mut self, info: &crate::NpcInfo, sheet: &arcanum_rules::CharacterSheet, block: crate::Block, base: u32, numbers: &[i32]) -> Option<String> {
        let vfs_guard = self.vfs.as_ref().and_then(|v| v.0.read().ok());
        let vfs = vfs_guard.as_deref();
        let who = crate::Speaker { npc: info, pc_name: &sheet.name, pc_female: sheet.gender == arcanum_rules::FEMALE, pc_race: sheet.race };
        let (text, _) = crate::npc_line(&mut self.generic, vfs, &mut self.rng, &who, block, base)?;
        Some(with_numbers(&text, numbers))
    }

    /// `player`'s window, as it now stands, with each item's line (0x578330):
    /// - the NPC's goods: `gd_npc` 800 with the price, and with haggle training the percentage
    ///   over worth (`(%d%%)`, at least 1); 900 when it will not sell;
    /// - the PC's: 1100 with the price, 1400 with the price and the NPC's gold when it has less;
    ///   1200 when it will not buy, 4600 for stolen goods.
    fn view(&mut self, player: PlayerId, sheet: &arcanum_rules::CharacterSheet, pc: &Inventory, open: &BarterOpen) -> Option<BarterView> {
        let (trader, haggle, training, npc) = self.deal_parts(player, sheet, open.sector, open.index)?;
        let stock = self.map.stock(open.stock)?;
        if open.pickpocket {
            return Some(BarterView { npc: open.npc, npc_male: npc.male, npc_name: npc.name.clone(), stock, buy: BTreeMap::new(), sell: BTreeMap::new(), npc_gold: 0, pickpocket: true });
        }
        let npc_gold = self.map.npc_inventory(open.sector, open.index).map_or(0, |i| i.gold());
        let vfs = self.vfs.as_ref().map(|v| v.0.clone());
        let vfs_guard = vfs.as_ref().and_then(|v| v.read().ok());
        let (prototypes, art) = (self.map.prototypes.clone(), self.map.art.clone());
        let info = crate::item_info(&prototypes, |id| crate::world::art_size(vfs_guard.as_deref(), art.as_deref(), id));
        let deal = Deal { info: &info, trader, haggle, training };
        let mut buy = BTreeMap::new();
        for item in &stock.items {
            let Some(price) = deal.price(item, Seller::Npc) else { continue };
            let line = if price == 0 {
                self.line(&npc, sheet, crate::Block::Npc, 900, &[])
            } else {
                let mut line = self.line(&npc, sheet, crate::Block::Npc, 800, &[price]).unwrap_or_default();
                if training > 0 {
                    let base = deal.worth(item).max(1);
                    line.push_str(&format!(" ({}%)", ((price - base) * 100 / base).max(1)));
                }
                Some(line)
            };
            buy.insert(item.location, Offer { price, line: line.unwrap_or_default() });
        }
        let mut sell = BTreeMap::new();
        for item in &pc.items {
            let Some(price) = deal.price(item, Seller::Pc) else { continue };
            let line = match price {
                0 if info(item.prototype).is_some_and(|i| i.item_flags & ITEM_STOLEN != 0) => self.line(&npc, sheet, crate::Block::Npc, 4600, &[]),
                0 => self.line(&npc, sheet, crate::Block::Npc, 1200, &[]),
                p if p > npc_gold => self.line(&npc, sheet, crate::Block::Npc, 1400, &[p, npc_gold]),
                p => self.line(&npc, sheet, crate::Block::Npc, 1100, &[p]),
            };
            sell.insert(item.location, Offer { price, line: line.unwrap_or_default() });
        }
        Some(BarterView { npc: open.npc, npc_male: npc.male, npc_name: npc.name.clone(), stock, buy, sell, npc_gold, pickpocket: false })
    }
}

/// Opens the barter windows asked for (0x572240 mode 1): the goods are the NPC's substitute
/// inventory if it has one in reach, whose gold then moves to the NPC (0x572640).
pub fn open_barters(mut commands: Commands, mut starts: ResMut<BarterStarts>, mut pockets: ResMut<crate::pickpocket::PickPocketStarts>, mut w: BarterWorld, pcs: Query<TraderQuery>) {
    // Pick-pocket windows: the NPC's own inventory, as it stands.
    for (player, npc, object) in std::mem::take(&mut pockets.0) {
        let ScriptObject::Mobile { sector, index } = object else { continue };
        let Some((entity, _, _, sheet, inventory, _)) = pcs.iter().find(|pc| pc.2.0 == player) else { continue };
        let open = BarterOpen { npc, sector, index, stock: Stock::Npc { sector, index }, pickpocket: true };
        commands.entity(entity).insert(open);
        if let Some(view) = w.view(player, &sheet.0, inventory, &open) {
            w.outbox.0.push((player, LedgerDelta::Barter { view }));
        }
    }
    for (player, npc, object) in std::mem::take(&mut starts.0) {
        let ScriptObject::Mobile { sector, index } = object else { continue };
        let Some((entity, _, _, sheet, inventory, _)) = pcs.iter().find(|pc| pc.2.0 == player) else { continue };
        let stock = match w.map.substitute(sector, index) {
            Some(container) => {
                w.restock(sector, index, Stock::Container(container));
                if let (Some(mut goods), Some(mut own)) = (w.map.stock(Stock::Container(container)), w.map.npc_inventory(sector, index)) {
                    let gold = goods.gold();
                    if gold > 0 {
                        goods.change_gold(-gold);
                        own.change_gold(gold);
                        w.map.set_stock(Stock::Container(container), goods);
                        w.map.set_npc_inventory(sector, index, own);
                        crate::ground::save_change(&mut w.world, &w.map, container.sector());
                        crate::ground::save_change(&mut w.world, &w.map, sector);
                    }
                }
                Stock::Container(container)
            }
            None => Stock::Npc { sector, index },
        };
        if stock == (Stock::Npc { sector, index }) {
            w.restock(sector, index, stock);
        }
        let open = BarterOpen { npc, sector, index, stock, pickpocket: false };
        commands.entity(entity).insert(open);
        if let Some(view) = w.view(player, &sheet.0, inventory, &open) {
            w.outbox.0.push((player, LedgerDelta::Barter { view }));
        }
    }
}

/// Buys and sells (0x575200 / 0x575360 → 0x5788c0 → 0x578ea0): each trade is done at once, gold
/// moving between the PC and the NPC.
/// - **Buying:** a price of 0 is "I do not want to sell that." (`gd_npc` 900); a PC without the
///   gold hears `gd_cls` 2000 (0x413910) and nothing moves.
/// - **Selling:** a price of 0 is `gd_npc` 1200, or 4600 for stolen goods; an NPC short of gold
///   pays what it has (0x575360).
///
/// Not built: the reaction change for over- and underpaying (0x4c11d0); stacks bought in part.
pub fn apply_barter(mut commands: Commands, mut requests: ResMut<BarterRequests>, mut w: BarterWorld, mut pcs: Query<TraderQuery>) {
    for (player, seq, request) in std::mem::take(&mut requests.0) {
        let Some((entity, &net_id, _, sheet, mut inventory, open)) = pcs.iter_mut().find(|pc| pc.2.0 == player) else { continue };
        let sheet = sheet.0.clone();
        let Some(open) = open.copied() else {
            w.outbox.0.push((player, LedgerDelta::Inventory { net_id, ack: seq, inventory: inventory.clone() }));
            continue;
        };
        if request == crate::InventoryRequest::CloseBarter {
            commands.entity(entity).remove::<BarterOpen>();
            w.outbox.0.push((player, LedgerDelta::Inventory { net_id, ack: seq, inventory: inventory.clone() }));
            continue;
        }
        let outcome = trade(&mut w, player, &sheet, &inventory, &open, &request);
        match outcome {
            Ok(changed) => {
                *inventory = changed;
                if let Some(saved) = w.world.0.as_mut().and_then(|s| s.players.get_mut(&player)) {
                    saved.inventory = Some(inventory.clone());
                }
            }
            Err(Refused::Line(text)) => w.outbox.0.push((player, LedgerDelta::BarterLine { text })),
            Err(Refused::Item(r)) => {
                if let Some(message) = r.message() {
                    w.outbox.0.push((player, LedgerDelta::ItemRefused { message }));
                }
            }
        }
        w.outbox.0.push((player, LedgerDelta::Inventory { net_id, ack: seq, inventory: inventory.clone() }));
        if let Some(view) = w.view(player, &sheet, &inventory, &open) {
            w.outbox.0.push((player, LedgerDelta::Barter { view }));
        }
    }
}

/// Why a trade did not happen.
enum Refused {
    /// The NPC says this in the window.
    Line(String),
    Item(crate::ItemRefusal),
}

fn trade(w: &mut BarterWorld, player: PlayerId, sheet: &arcanum_rules::CharacterSheet, pc: &Inventory, open: &BarterOpen, request: &crate::InventoryRequest) -> Result<Inventory, Refused> {
    use crate::InventoryRequest::{Buy, Sell};
    let silent = Refused::Item(crate::ItemRefusal::Silent);
    if open.pickpocket {
        let Buy { from, to } = *request else { return Err(silent) };
        return steal(w, player, sheet, pc, open, from, to);
    }
    let (trader, haggle, training, npc) = w.deal_parts(player, sheet, open.sector, open.index).ok_or(Refused::Item(crate::ItemRefusal::Silent))?;
    let vfs = w.vfs.as_ref().map(|v| v.0.clone());
    let vfs_guard = vfs.as_ref().and_then(|v| v.read().ok());
    let (prototypes, art) = (w.map.prototypes.clone(), w.map.art.clone());
    let info = crate::item_info(&prototypes, |id| crate::world::art_size(vfs_guard.as_deref(), art.as_deref(), id));
    let deal = Deal { info: &info, trader, haggle, training };
    let stock = w.map.stock(open.stock).ok_or(Refused::Item(crate::ItemRefusal::Silent))?;
    let wearer = crate::Wearer { race: sheet.race, gender: sheet.gender };
    let (carry, aptitude) = (w.rules.0.derived(sheet).carry_weight, w.rules.0.aptitude(sheet));
    let own_stock = matches!(open.stock, Stock::Npc { .. });
    match *request {
        Buy { from, to } => {
            let (rest, item) = stock.take_out(from).ok_or(silent)?;
            let price = deal.price(&item, Seller::Npc).ok_or(Refused::Item(crate::ItemRefusal::Silent))?;
            if price == 0 {
                return Err(Refused::Line(w.line(&npc, sheet, crate::Block::Npc, 900, &[]).unwrap_or_default()));
            }
            if pc.gold() < price {
                return Err(Refused::Line(w.line(&npc, sheet, crate::Block::Class, 2000, &[price]).unwrap_or_default()));
            }
            let mut bought = pc.put_in(&CarriedItem { location: 0, ..item }, to, wearer, &info, carry, aptitude).map_err(Refused::Item)?;
            bought.change_gold(-price);
            let mut rest = rest;
            if own_stock {
                rest.change_gold(price);
            } else if let Some(mut own) = w.map.npc_inventory(open.sector, open.index) {
                own.change_gold(price);
                w.map.set_npc_inventory(open.sector, open.index, own);
            }
            w.map.set_stock(open.stock, rest);
            save_stock(w, open);
            Ok(bought)
        }
        Sell { from, to } => {
            let (sold, item) = pc.take_out(from).ok_or(silent)?;
            let price = deal.price(&item, Seller::Pc).ok_or(Refused::Item(crate::ItemRefusal::Silent))?;
            if price == 0 {
                let stolen = info(item.prototype).is_some_and(|i| i.item_flags & ITEM_STOLEN != 0);
                let base = if stolen { 4600 } else { 1200 };
                return Err(Refused::Line(w.line(&npc, sheet, crate::Block::Npc, base, &[]).unwrap_or_default()));
            }
            let npc_gold = w.map.npc_inventory(open.sector, open.index).map_or(0, |i| i.gold());
            let price = price.min(npc_gold);
            let npc_wearer = crate::Wearer { race: 0, gender: arcanum_rules::MALE };
            let to = to.filter(|&t| stock.in_grid(t));
            let mut goods = stock.put_in(&CarriedItem { location: 0, ..item }, to, npc_wearer, &info, i32::MAX, 0).map_err(Refused::Item)?;
            if own_stock {
                goods.change_gold(-price);
            } else if let Some(mut own) = w.map.npc_inventory(open.sector, open.index) {
                own.change_gold(-price);
                w.map.set_npc_inventory(open.sector, open.index, own);
            }
            w.map.set_stock(open.stock, goods);
            save_stock(w, open);
            let mut sold = sold;
            sold.change_gold(price);
            Ok(sold)
        }
        _ => Err(silent),
    }
}

/// One steal ([`crate::pickpocket`]): the roll, then the item (gold and ammunition in part) to
/// the PC, or a failure the victim may notice.
#[allow(clippy::too_many_arguments)]
fn steal(w: &mut BarterWorld, player: PlayerId, sheet: &arcanum_rules::CharacterSheet, pc: &Inventory, open: &BarterOpen, from: i32, to: Option<i32>) -> Result<Inventory, Refused> {
    use crate::pickpocket::{MOST_AMMO, MOST_GOLD, SKILL_PICK_POCKET, STEAL_FAILED, STEAL_SUCCEEDED};
    let silent = || Refused::Item(crate::ItemRefusal::Silent);
    let vfs = w.vfs.as_ref().map(|v| v.0.clone());
    let vfs_guard = vfs.as_ref().and_then(|v| v.read().ok());
    let (prototypes, art) = (w.map.prototypes.clone(), w.map.art.clone());
    let info = crate::item_info(&prototypes, |id| crate::world::art_size(vfs_guard.as_deref(), art.as_deref(), id));
    let stock = w.map.stock(open.stock).ok_or_else(silent)?;
    let (mut rest, mut item) = stock.take_out(from).ok_or_else(silent)?;
    let victim = w.map.mobiles.get(&open.sector).and_then(|m| m.get(open.index as usize)).ok_or_else(silent)?;
    let perception = crate::pickpocket::perception(victim, &prototypes);
    let chance = crate::pickpocket::steal_chance(w.rules.0.skill_level(sheet, SKILL_PICK_POCKET), perception);
    let kind = info(item.prototype);
    let training = sheet.training[SKILL_PICK_POCKET];
    let difficulty = crate::pickpocket::steal_difficulty(from, kind.map_or((1, 1), |k| k.cells), training);
    let outcome = crate::roll_pick(chance, difficulty, &mut w.rng);
    if outcome != crate::PickOutcome::Success {
        w.outbox.0.push((player, LedgerDelta::SkillResult { message: STEAL_FAILED }));
        let noticed = outcome == crate::PickOutcome::Jammed || (training == 0 && w.rng.range(1, 20) <= perception);
        if noticed {
            w.caught.0.push((player, ScriptObject::Mobile { sector: open.sector, index: open.index }));
        }
        return Err(silent());
    }
    // Gold and ammunition: a handful at a time (0x464830, 0x4658e0).
    let most = if item.prototype == crate::GOLD { MOST_GOLD } else if kind.is_some_and(|k| k.kind == arcanum_formats::object::TYPE_AMMO) { MOST_AMMO } else { i32::MAX };
    if item.quantity > most {
        rest.items.push(CarriedItem { quantity: item.quantity - most, ..item.clone() });
        item.quantity = most;
    }
    let wearer = crate::Wearer { race: sheet.race, gender: sheet.gender };
    let (carry, aptitude) = (w.rules.0.derived(sheet).carry_weight, w.rules.0.aptitude(sheet));
    let taken = pc.put_in(&CarriedItem { location: 0, ..item }, to, wearer, &info, carry, aptitude).map_err(Refused::Item)?;
    w.map.set_stock(open.stock, rest);
    save_stock(w, open);
    w.outbox.0.push((player, LedgerDelta::SkillResult { message: STEAL_SUCCEEDED }));
    Ok(taken)
}

fn save_stock(w: &mut BarterWorld, open: &BarterOpen) {
    crate::ground::save_change(&mut w.world, &w.map, open.sector);
    if let Stock::Container(id) = open.stock {
        crate::ground::save_change(&mut w.world, &w.map, id.sector());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sword(worth: i32) -> ItemInfo {
        ItemInfo { kind: arcanum_formats::object::TYPE_WEAPON, inventory_art: 0, cells: (1, 3), weight: 1, magic_weight_adj: 0, complexity: 0, generic_flags: 0, weapon_flags: 0, armor_flags: 0, item_flags: 0, worth }
    }

    #[test]
    fn haggle_and_reaction_follow_the_original() {
        assert_eq!(haggle(0, 8), 10);
        assert_eq!(haggle(20, 20), 95, "capped");
        assert_eq!(haggle(8, 20), 52);
        assert_eq!(reaction_factor(-5), 200);
        assert_eq!(reaction_factor(30), 140);
        assert_eq!(reaction_factor(50), 100);
        assert_eq!(reaction_factor(80), 88);
        assert_eq!(reaction_factor(150), 80);
    }

    #[test]
    fn prices_for_buying_and_selling() {
        // A shopkeeper (multiplier 300) at reaction 50, a PC of haggle 10: 100 + 90 × 300 × 100 / 10000.
        assert_eq!(buy_price(100, 10, 300, 50), 370);
        assert_eq!(buy_price(100, 10, 300, 80), 370 * 88 / 100);
        assert_eq!(buy_price(100, 10, 0, 80), 101, "never at or under the worth");
        assert_eq!(buy_price(2, 95, 0, 100), 3);
        // Selling: multiplier 300 lowers the worth by 75 %, (10 / 2 + 50) % of what is left.
        assert_eq!(sell_price(100, 10, 300, 1, 1), 55 * 25 / 100);
        assert_eq!(sell_price(100, 10, 100, 1, 1), 55);
        assert_eq!(sell_price(100, 10, 100, 5, 10), 27, "half broken");
        assert_eq!(sell_price(2, 10, 100, 1, 1), 2, "1 becomes 2");
    }

    #[test]
    fn worth_refusals_and_gold() {
        let mut magic = sword(1000);
        magic.complexity = 40;
        assert_eq!(worth(&magic, 0, 1), UNKNOWN_WORTH);
        assert_eq!(worth(&magic, ITEM_IDENTIFIED, 1), 1000);
        assert_eq!(worth(&sword(0), 0, 1), 2);
        let trader = Trader { multiplier: 100, flags: 0, reaction: 50 };
        assert_eq!(price(&sword(100), 0, 1, true, Seller::Npc, trader, 10, 2), 0, "worn: Masters only");
        assert_eq!(price(&sword(100), 0, 1, true, Seller::Npc, trader, 10, MASTER), 190);
        assert_eq!(price(&sword(100), ITEM_STOLEN, 1, false, Seller::Pc, trader, 10, 0), 0, "no fence");
        assert_eq!(price(&sword(100), ITEM_STOLEN, 1, false, Seller::Pc, Trader { flags: NPC_FENCE, ..trader }, 10, 0), 55);
        assert_eq!(price(&sword(0), 0, 1, false, Seller::Pc, trader, 10, 0), 0, "worthless");
        let gold = ItemInfo { kind: TYPE_GOLD, ..sword(1) };
        assert_eq!(price(&gold, 0, 250, false, Seller::Npc, trader, 10, 0), 250);
    }
}
