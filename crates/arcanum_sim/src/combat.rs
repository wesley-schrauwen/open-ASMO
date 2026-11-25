//! Turn-based combat (`plans/combat.md`; Arcanum.exe combat module 0x4b3xxx-0x4b8xxx, attack
//! 0x4b3170, damage 0x4b6680 / 0x4b6860 / 0x4b4390).
//!
//! - **The encounter** ([`Combat`]): a PC's attack starts one round it (0x4b71e0): the PCs and
//!   the fighting NPCs within [`ENGAGE_RADIUS`] tiles, PCs first, NPCs after in the order found
//!   (0x4b7eb0 sorts by nothing else). Each round the list is gathered again (0x4b73a0); a round
//!   with no fighting NPC left ends it. PCs outside the radius move freely (owner's rule).
//! - **Turns and action points** (0x4b6e70): `max(5, speed)` at the start of a turn, none kept.
//!   A step costs [`STEP_AP`] (walking, 0x4b7ba0); an attack its weapon's cost
//!   ([`attack_cost`], 0x4b7c30). Out of points, or `PlayerIntent::EndTurn` (0x4b7c90), the next
//!   critter's turn starts. A fight with no NPC fighting or no PC standing ends at once.
//! - **NPC turns** ([`npc_turn`]): at the start, flee when hurt to its `ai_params.mes` row's
//!   percent ([`should_flee`]); else attack its target (or the nearest standing PC of the fight),
//!   walking next to it first, until out of points.
//! - **The attack** ([`resolve`], 0x4b3170): see the function. A hit makes an NPC victim fight
//!   its attacker (0x4a9650) and join the encounter (0x4b7e00).
//! - **Damage to NPCs** is kept as `hp_damage` in the sector's changes; at 0 hit points they die
//!   as action 0x3a kills (0x45da20), and the experience is shared (team rule).
//!
//! - **PCs hurt:** an NPC's hit goes on the PC's sheet (hit points, poison, fatigue) as script
//!   damage does; a PC with no hit points left takes no turns.
//!
//! Not built yet: allies joining (0x4a9e10), surrender, fleeing weighed against the target
//! (columns 1-3), critical effects other than the multiplier, called shots, the light, range and
//! cover terms, ammunition, PC death.

use arcanum_formats::map::TileLoc;
use arcanum_formats::object::{Object, Prototypes};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{HostRng, LedgerDelta, NetId, PlayerId};

/// Who is gathered into a fight, in tiles (`0x5b57b8`).
pub const ENGAGE_RADIUS: u32 = 15;
/// The fewest action points a turn starts with (0x4b6e70).
pub const MIN_AP: i32 = 5;
/// Action points a walked step costs (0x4b7ba0: path cost × 2).
pub const STEP_AP: i32 = 2;
/// The weapon slot.
pub const WEAPON_SLOT: i32 = 1004;
/// The gloves slot (unarmed bonus damage, field 0x9e).
pub const GLOVES_SLOT: i32 = 1007;
/// Damage types: normal, poison, electrical, fire, fatigue.
pub const DAMAGE_TYPES: usize = 5;
/// The resistance each damage type is cut by (table 0x5b5804: damage, poison, electrical, fire,
/// damage).
const RESISTANCE_OF: [usize; DAMAGE_TYPES] = [0, 3, 2, 1, 0];
/// Skills: bow, dodge, melee, throwing; firearms is tech skill 1 (13).
pub const SKILL_BOW: usize = 0;
pub const SKILL_DODGE: usize = 1;
pub const SKILL_MELEE: usize = 2;
pub const SKILL_THROWING: usize = 3;
pub const SKILL_FIREARMS: usize = 13;
/// The highest chance.
pub const MAX_CHANCE: i32 = 95;

/// A skill's chance at level `level` (0x4c62e0 / 0x4c69f0): dodge 2 a level, throwing 7 + 25,
/// bow and melee 5 + 25, firearms (tech) 6; at most 95.
pub fn weapon_chance(skill: usize, level: i32) -> i32 {
    let level = level.max(0);
    let chance = match skill {
        SKILL_DODGE => 2 * level,
        SKILL_THROWING => 7 * level + 25,
        SKILL_FIREARMS => level * 6,
        _ => 5 * level + 25,
    };
    chance.clamp(0, MAX_CHANCE)
}

/// An attack's action points for a weapon of speed `speed` (0x4b7c30).
pub fn attack_cost(speed: i32) -> i32 {
    if speed > 20 { if speed <= 24 { 2 } else { 1 } } else { (8 - speed / 3).max(1) }
}

/// What an attacker strikes with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Weapon {
    /// The skill it uses.
    pub skill: usize,
    pub to_hit: i32,
    pub damage: [(i32, i32); DAMAGE_TYPES],
    /// The magick part of each damage, once identified.
    pub magic_damage: [i32; DAMAGE_TYPES],
    pub speed: i32,
    /// In tiles; 1 or less for melee.
    pub range: i32,
    pub min_strength: i32,
}

/// One side of an attack.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fighter {
    /// Its skill levels by skill (bow, dodge, melee, throwing, firearms as 13).
    pub levels: Vec<(usize, i32)>,
    pub strength: i32,
    /// Stat 9.
    pub damage_bonus: i32,
    /// `None`: unarmed.
    pub weapon: Option<Weapon>,
    /// Unarmed damage per type (PCs 1-5 normal and fatigue, more with gloves; critters
    /// `npc_damage_idx`).
    pub unarmed: [(i32, i32); DAMAGE_TYPES],
    /// Resistances in percent: damage, fire, electrical, poison, magick.
    pub resistances: [i32; 5],
    /// The skills it is trained in (a weapon's speed + 5, 0x465cf0).
    pub trained: Vec<usize>,
}

impl Fighter {
    fn level(&self, skill: usize) -> i32 {
        self.levels.iter().find(|l| l.0 == skill).map_or(0, |l| l.1)
    }

    /// The skill its attacks use.
    pub fn skill(&self) -> usize {
        self.weapon.as_ref().map_or(SKILL_MELEE, |w| w.skill)
    }

    /// Its weapon's speed (0x465cf0): bare hands 10; a weapon's, + 5 when trained in its skill.
    pub fn speed(&self) -> i32 {
        match &self.weapon {
            None => UNARMED_SPEED,
            Some(w) => w.speed + if self.trained.contains(&w.skill) { 5 } else { 0 },
        }
    }

    /// An attack's action points.
    pub fn attack_cost(&self) -> i32 {
        attack_cost(self.speed())
    }

    /// How far it reaches, in tiles.
    pub fn reach(&self) -> u32 {
        self.weapon.as_ref().map_or(1, |w| w.range.max(1) as u32)
    }
}

/// How an attack went.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttackOutcome {
    pub hit: bool,
    pub critical: bool,
    pub dodged: bool,
    /// A critical miss (0x4c82e0); its effects are not built.
    pub fumbled: bool,
    /// Hit points, poison and fatigue taken.
    pub hp: i32,
    pub poison: i32,
    pub fatigue: i32,
    /// The critical multiplier in percent (150, 200, 300), 100 for none.
    pub multiplier: i32,
}

/// One attack (0x4b3170 → 0x4c7160 → 0x4b6680 → 0x4b6860 → 0x4b4390):
/// - **To hit:** 1-100 plus the difficulty at or under the weapon skill's chance; the difficulty
///   is −(the weapon's to-hit) plus 5 a point of Strength short of the weapon's. A hit is critical
///   when a second roll is at or under half the chance; a miss fumbles when one is at or under
///   `max((100 − chance) / 2, 2)` (as Pick Locks').
/// - **Dodge:** a hit is dodged when the defender's Dodge chance (2 a level) makes its roll.
/// - **Damage:** each type rolled between its bounds (the weapon's plus its magick part; unarmed,
///   the attacker's own); melee adds the damage bonus to normal and fatigue damage; each type
///   less its resistance in percent (fatigue three quarters of it). A critical multiplies the
///   hit points: × 3 one time in ten, × 2 two in ten, else × 1.5 (0x4b5f40 with no chart
///   bonus).
pub fn resolve(attacker: &Fighter, defender: &Fighter, rng: &mut HostRng) -> AttackOutcome {
    let skill = attacker.skill();
    let chance = weapon_chance(skill, attacker.level(skill));
    let weapon = attacker.weapon.clone().unwrap_or_default();
    let short = (weapon.min_strength - attacker.strength).max(0);
    let difficulty = -weapon.to_hit + 5 * short;
    let mut out = AttackOutcome { multiplier: 100, ..Default::default() };
    if rng.range(1, 100) + difficulty > chance {
        out.fumbled = rng.range(1, 100) <= ((100 - chance) / 2).max(2);
        return out;
    }
    out.critical = rng.range(1, 100) <= chance / 2;
    let dodge = weapon_chance(SKILL_DODGE, defender.level(SKILL_DODGE));
    if dodge > 0 && rng.range(1, 100) <= dodge {
        out.dodged = true;
        return out;
    }
    out.hit = true;
    let melee = weapon.range <= 1;
    let mut dealt = [0; DAMAGE_TYPES];
    for (i, dealt) in dealt.iter_mut().enumerate() {
        let (mut lo, mut hi) = match &attacker.weapon {
            Some(w) => (w.damage[i].0 + w.magic_damage[i], w.damage[i].1 + w.magic_damage[i]),
            None => attacker.unarmed[i],
        };
        if melee && (i == 0 || i == 4) && hi > 0 {
            lo = (lo + attacker.damage_bonus).max(1);
            hi = (hi + attacker.damage_bonus).max(1);
        }
        if hi <= 0 {
            continue;
        }
        let rolled = rng.range(lo.max(0).min(hi), hi);
        let r = defender.resistances[RESISTANCE_OF[i]];
        let r = if i == 4 { r * 3 / 4 } else { r };
        *dealt = if r > 0 { rolled - rolled * r.min(100) / 100 } else { rolled };
    }
    if out.critical {
        let roll = rng.range(1, 100);
        out.multiplier = if roll <= 10 { 300 } else if roll <= 30 { 200 } else { 150 };
    }
    out.hp = (dealt[0] + dealt[2] + dealt[3]) * out.multiplier / 100;
    out.poison = dealt[1];
    out.fatigue = dealt[4];
    out
}

/// A weapon item's fighting stats (fields 0x72-0x7d) and its skill (0x465d80): thrown with
/// `weapon_flags` 0x40; firearms when tech, of range 3 or more, firing bullets, charges or fuel
/// (ammunition 1-3); a bow when it shoots arrows (0); else melee.
pub fn weapon_of(item: &Object, prototypes: &Prototypes) -> Weapon {
    let int = |f: &str| prototypes.int(item, f).unwrap_or(0);
    let at = |f: &str, i: usize| item.ints(f).or_else(|| prototypes.of(item)?.ints(f)).and_then(|v| v.get(i).copied()).unwrap_or(0);
    let range = int("weapon_range");
    let ammo = int("weapon_ammo_type");
    let skill = if int("weapon_flags") & WEAPON_THROWN != 0 {
        SKILL_THROWING
    } else if int("item_magic_tech_complexity") < 0 && range >= 3 && (1..=3).contains(&ammo) {
        SKILL_FIREARMS
    } else if ammo == 0 {
        SKILL_BOW
    } else {
        SKILL_MELEE
    };
    let mut damage = [(0, 0); DAMAGE_TYPES];
    let mut magic = [0; DAMAGE_TYPES];
    for i in 0..DAMAGE_TYPES {
        damage[i] = (at("weapon_damage_lower_idx", i), at("weapon_damage_upper_idx", i));
        magic[i] = at("weapon_magic_damage_adj_idx", i);
    }
    Weapon {
        skill,
        to_hit: int("weapon_bonus_to_hit") + int("weapon_magic_hit_adj"),
        damage,
        magic_damage: magic,
        speed: int("weapon_speed_factor") + int("weapon_magic_speed_adj"),
        range: if range == 1 { 0 } else { range + int("weapon_magic_range_adj") },
        min_strength: int("weapon_min_strength") + int("weapon_magic_min_strength_adj"),
    }
}

/// `weapon_flags`: a throwing weapon (0x465d80).
pub const WEAPON_THROWN: i32 = 0x40;
/// A bare hand's speed (0x465cf0 with no weapon).
pub const UNARMED_SPEED: i32 = 10;

/// An unarmed PC's damage (0x465f70 with no weapon): normal and fatigue `max(g − 25, 1)` to
/// `g + 5`, `g` the gloves' `armor_unarmed_bonus_damage`.
pub fn pc_unarmed(gloves: i32) -> [(i32, i32); DAMAGE_TYPES] {
    let hi = gloves + 5;
    let lo = (hi - 30).max(1);
    [(lo, hi), (0, 0), (0, 0), (0, 0), (lo, hi)]
}

/// A critter's own damage (`npc_damage_idx`, pairs by type).
pub fn npc_unarmed(object: &Object, prototypes: &Prototypes) -> [(i32, i32); DAMAGE_TYPES] {
    let list = object.ints("npc_damage_idx").or_else(|| prototypes.of(object)?.ints("npc_damage_idx")).unwrap_or_default();
    let mut out = [(0, 0); DAMAGE_TYPES];
    for (i, o) in out.iter_mut().enumerate() {
        *o = (list.get(2 * i).copied().unwrap_or(0), list.get(2 * i + 1).copied().unwrap_or(0));
    }
    if out.iter().all(|d| d.1 <= 0) { pc_unarmed(0) } else { out }
}

/// Resistances from worn armour: each `armor_resistance_adj_idx` plus its magick part, summed,
/// at most 95 (builder's call: 0x43d6d0 was not read).
pub fn armour_resistances(worn: &[&Object], prototypes: &Prototypes) -> [i32; 5] {
    let mut out = [0; 5];
    for item in worn {
        let at = |f: &str, i: usize| item.ints(f).or_else(|| prototypes.of(item)?.ints(f)).and_then(|v| v.get(i).copied()).unwrap_or(0);
        for (i, r) in out.iter_mut().enumerate() {
            *r += at("armor_resistance_adj_idx", i) + at("armor_magic_resistance_adj_idx", i);
        }
    }
    out.map(|r| r.min(MAX_CHANCE))
}

/// An NPC's stat (its `critter_stat_base_idx`).
pub fn npc_stat(object: &Object, prototypes: &Prototypes, stat: usize) -> i32 {
    object.ints("critter_stat_base_idx").or_else(|| prototypes.of(object)?.ints("critter_stat_base_idx")).and_then(|s| s.get(stat).copied()).unwrap_or(0)
}

/// An NPC's skill level: points × 4, at most 20.
pub fn npc_skill_level(object: &Object, prototypes: &Prototypes, k: usize) -> i32 {
    let (field, i) = if k < 12 { ("critter_basic_skill_idx", k) } else { ("critter_tech_skill_idx", k - 12) };
    let raw = object.ints(field).or_else(|| prototypes.of(object)?.ints(field)).and_then(|v| v.get(i).copied()).unwrap_or(0);
    ((raw & 0x3f) * arcanum_rules::LEVELS_PER_POINT).min(arcanum_rules::MAX_SKILL_LEVEL)
}

/// An NPC's most hit points (0x43d5a0): `4 × hp_pts + hp_adj + WP + 2 × (level + ST) + 4`.
pub fn npc_max_hp(object: &Object, prototypes: &Prototypes) -> i32 {
    let stat = |i| npc_stat(object, prototypes, i);
    4 * prototypes.int(object, "hp_pts").unwrap_or(0) + prototypes.int(object, "hp_adj").unwrap_or(0) + stat(6) + 2 * (stat(17) + stat(0)) + 4
}

/// An NPC's speed (stat 11 from Dexterity, as a PC's).
pub fn npc_speed(object: &Object, prototypes: &Prototypes) -> i32 {
    let dx = npc_stat(object, prototypes, 1);
    dx + if dx >= 20 { 5 } else { 0 }
}

/// An NPC's damage bonus (stat 9 from Strength, as a PC's).
pub fn damage_bonus(st: i32) -> i32 {
    let d = st - 10;
    let d = if d < 0 { d / 2 } else { d };
    if st >= 20 { d * 2 } else { d }
}

/// The share of an NPC's experience a hit earns (0x4b4390): its percent of the most hit points
/// of `worth`, at most what is left of the pool.
pub fn hit_experience(damage: i32, max_hp: i32, worth: i32, pool: i32) -> i32 {
    if max_hp <= 0 || pool <= 0 {
        return 0;
    }
    (damage.max(0) * 100 / max_hp * worth / 100).min(pool)
}

/// An NPC fighting someone (0x118 bit 1, its target in field 0x11b).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fighting {
    pub target: NetId,
}

/// The running fight, if any.
#[derive(Resource, Debug, Default)]
pub struct Combat(pub Option<Encounter>);

/// Who fights, in turn order, whose turn it is and its points left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encounter {
    pub order: Vec<NetId>,
    pub turn: usize,
    pub ap: i32,
    pub round: u32,
    /// The turn has just started (an NPC decides whether to flee).
    pub fresh: bool,
}

impl Encounter {
    pub fn current(&self) -> Option<NetId> {
        self.order.get(self.turn).copied()
    }

    /// What clients see of it.
    pub fn delta(&self) -> LedgerDelta {
        LedgerDelta::Combat { order: self.order.clone(), turn: self.current(), ap: self.ap }
    }
}

/// Attack and end-turn requests taken from the inbox.
#[derive(Resource, Debug, Default)]
pub struct CombatRequests {
    pub attacks: Vec<(PlayerId, NetId, u32)>,
    pub end_turn: Vec<PlayerId>,
}

/// Fights started elsewhere (dialogue `co`, script action 0x24): the NPC and whom it attacks.
#[derive(Resource, Debug, Default)]
pub struct CombatStarts(pub Vec<(NetId, NetId)>);

/// Script events the fight raises (0x4aabe0, 0x45da20): the NPC, the event and the player whose
/// PC triggers it.
#[derive(Resource, Debug, Default)]
pub struct CombatEvents(pub Vec<(crate::ScriptObject, u32, PlayerId)>);

/// Script events: an NPC dying (killer the triggerer), starting and stopping a fight.
pub const EVENT_DYING: u32 = 12;
pub const EVENT_ENTER_COMBAT: u32 = 13;
pub const EVENT_EXIT_COMBAT: u32 = 14;

/// Runs the fight's script events with the host.
pub fn run_combat_events(mut events: ResMut<CombatEvents>, mut host: crate::ScriptHost, mut world: ResMut<crate::WorldState>, mut map: ResMut<crate::LoadedMap>, mut outbox: ResMut<crate::Outbox>) {
    for (object, event, player) in std::mem::take(&mut events.0) {
        host.hook(&mut world, &mut map, &mut outbox, object, event, player);
    }
}

/// A PC's attack waiting for its walk to end next to the target.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingAttack {
    pub target: NetId,
}

/// The larger axis difference between two tiles (0x4b96f0).
pub fn reach_between(a: TileLoc, b: TileLoc) -> u32 {
    a.x.abs_diff(b.x).max(a.y.abs_diff(b.y))
}

impl crate::LoadedMap {
    /// NPC `index`'s hit points: what is left, and the most.
    pub fn npc_hp(&self, sector: arcanum_formats::map::SectorId, index: u32) -> Option<(i32, i32)> {
        let o = self.mobiles.get(&sector)?.get(index as usize)?;
        let max = npc_max_hp(o, &self.prototypes);
        Some((max - self.prototypes.int(o, "hp_damage").unwrap_or(0), max))
    }

    /// Adds `amount` to NPC `index`'s `hp_damage`, kept in [`crate::LoadedMap::changes`].
    pub fn hurt_npc(&mut self, sector: arcanum_formats::map::SectorId, index: u32, amount: i32) {
        let Some(o) = self.mobiles.get_mut(&sector).and_then(|m| m.get_mut(index as usize)) else { return };
        let now = o.int("hp_damage").unwrap_or(0) + amount.max(0);
        o.set_int("hp_damage", now);
        self.changes.entry(sector).or_default().npc_damage.insert(index, now);
    }

    /// Takes `amount` from NPC `index`'s experience pool (field 0x11e), kept the same way.
    pub fn spend_npc_pool(&mut self, sector: arcanum_formats::map::SectorId, index: u32, amount: i32) {
        let Some(o) = self.mobiles.get_mut(&sector).and_then(|m| m.get_mut(index as usize)) else { return };
        let pool = self.prototypes.int(o, "npc_experience_pool").unwrap_or(0) - amount;
        o.set_int("npc_experience_pool", pool);
        self.changes.entry(sector).or_default().npc_pools.insert(index, pool);
    }

    /// Kills NPC `index` (action 0x3a's way): dead, in its death frame.
    pub fn kill_npc(&mut self, vfs: Option<&arcanum_vfs::Vfs>, sector: arcanum_formats::map::SectorId, index: u32) {
        if self.is_dead(sector, index) {
            return;
        }
        self.changes.entry(sector).or_default().dead.insert(index);
        if let Some(art) = self.object_at(sector, index, true).and_then(|o| self.prototypes.art_id(o)) {
            let dying = crate::death_frame(vfs, self.art.as_deref(), art);
            self.set_object_art(sector, index, true, dying.0);
        }
    }
}

/// What the combat systems need of the world.
#[derive(bevy_ecs::system::SystemParam)]
pub struct CombatWorld<'w> {
    pub combat: ResMut<'w, Combat>,
    pub requests: ResMut<'w, CombatRequests>,
    pub map: ResMut<'w, crate::LoadedMap>,
    pub world: ResMut<'w, crate::WorldState>,
    pub session: Res<'w, crate::Session>,
    pub outbox: ResMut<'w, crate::Outbox>,
    pub rules: Res<'w, crate::SimRules>,
    pub rng: ResMut<'w, HostRng>,
    pub awards: ResMut<'w, crate::ExperienceAwards>,
    pub toggled: ResMut<'w, crate::world::ToggledNpcs>,
    pub vfs: Option<Res<'w, crate::SimVfs>>,
    pub sheets: ResMut<'w, crate::ScriptSheets>,
    pub ai: ResMut<'w, AiParams>,
    pub starts: ResMut<'w, CombatStarts>,
    pub events: ResMut<'w, CombatEvents>,
}

/// A critter as the combat systems see it.
type CombatantQuery<'a> = (Entity, &'a NetId, &'a crate::Position, Option<&'a crate::PlayerControlled>, Option<&'a crate::NpcSource>, Option<&'a crate::Sheet>, Option<&'a crate::Inventory>, Option<&'a Fighting>, Has<crate::Walking>, Option<&'a PendingAttack>);

/// One critter of the query, copied out.
#[derive(Clone)]
struct Seen {
    entity: Entity,
    net_id: NetId,
    at: TileLoc,
    player: Option<PlayerId>,
    npc: Option<crate::NpcSource>,
    sheet: Option<arcanum_rules::CharacterSheet>,
    inventory: Option<crate::Inventory>,
    fighting: bool,
    /// Whom it fights.
    target: Option<NetId>,
    walking: bool,
    pending: Option<PendingAttack>,
}

fn seen(q: &Query<CombatantQuery>) -> Vec<Seen> {
    q.iter()
        .map(|(entity, &net_id, at, pc, npc, sheet, inventory, fighting, walking, pending)| Seen {
            target: fighting.map(|f| f.target),
            entity,
            net_id,
            at: at.0,
            player: pc.map(|p| p.0),
            npc: npc.copied(),
            sheet: sheet.map(|s| s.0.clone()),
            inventory: inventory.cloned(),
            fighting: fighting.is_some(),
            walking,
            pending: pending.copied(),
        })
        .collect()
}

impl CombatWorld<'_> {
    /// A critter's fighting side.
    fn fighter(&self, c: &Seen) -> Fighter {
        let prototypes = &self.map.prototypes;
        let item = |inv: &crate::Inventory, slot: i32| inv.items.iter().find(|i| i.location == slot).and_then(|i| prototypes.get(i.prototype));
        if let (Some(sheet), Some(inv)) = (&c.sheet, &c.inventory) {
            let rules = &self.rules.0;
            let levels = [SKILL_BOW, SKILL_DODGE, SKILL_MELEE, SKILL_THROWING, SKILL_FIREARMS].map(|k| (k, rules.skill_level(sheet, k))).to_vec();
            let worn: Vec<&Object> = (crate::FIRST_SLOT..=crate::LAST_SLOT).filter_map(|s| item(inv, s)).filter(|o| o.object_type == arcanum_formats::object::TYPE_ARMOR).collect();
            let gloves = item(inv, GLOVES_SLOT).and_then(|g| prototypes.int(g, "armor_unarmed_bonus_damage")).unwrap_or(0);
            return Fighter {
                levels,
                strength: rules.stats(sheet)[0],
                damage_bonus: rules.derived(sheet).damage_bonus,
                weapon: item(inv, WEAPON_SLOT).filter(|w| w.object_type == arcanum_formats::object::TYPE_WEAPON).map(|w| weapon_of(w, prototypes)),
                unarmed: pc_unarmed(gloves),
                resistances: armour_resistances(&worn, prototypes),
                trained: [SKILL_BOW, SKILL_MELEE, SKILL_THROWING, SKILL_FIREARMS].into_iter().filter(|&k| sheet.training[k] > 0).collect(),
            };
        }
        let Some(source) = c.npc else { return Fighter::default() };
        let Some(o) = self.map.mobiles.get(&source.sector).and_then(|m| m.get(source.index as usize)) else { return Fighter::default() };
        let inv = self.map.npc_inventory(source.sector, source.index).unwrap_or_default();
        let worn: Vec<&Object> = (crate::FIRST_SLOT..=crate::LAST_SLOT).filter_map(|s| item(&inv, s)).filter(|o| o.object_type == arcanum_formats::object::TYPE_ARMOR).collect();
        Fighter {
            levels: [SKILL_BOW, SKILL_DODGE, SKILL_MELEE, SKILL_THROWING, SKILL_FIREARMS].map(|k| (k, npc_skill_level(o, prototypes, k))).to_vec(),
            strength: npc_stat(o, prototypes, 0),
            damage_bonus: damage_bonus(npc_stat(o, prototypes, 0)),
            weapon: item(&inv, WEAPON_SLOT).filter(|w| w.object_type == arcanum_formats::object::TYPE_WEAPON).map(|w| weapon_of(w, prototypes)),
            unarmed: npc_unarmed(o, prototypes),
            resistances: armour_resistances(&worn, prototypes),
            trained: Vec::new(),
        }
    }

    /// A turn's action points: `max(5, speed)` (0x4b6e70).
    fn turn_ap(&self, c: &Seen) -> i32 {
        let speed = match (&c.sheet, c.npc) {
            (Some(sheet), _) => self.rules.0.derived(sheet).speed,
            (None, Some(n)) => self.map.mobiles.get(&n.sector).and_then(|m| m.get(n.index as usize)).map_or(0, |o| npc_speed(o, &self.map.prototypes)),
            _ => 0,
        };
        speed.max(MIN_AP)
    }

    /// A PC with no hit points left (it takes no turns; death is step 4).
    fn pc_down(&self, c: &Seen) -> bool {
        let Some(p) = c.player else { return false };
        self.world.0.as_ref().and_then(|w| w.players.get(&p)).is_some_and(|s| s.blueprint.sheet.hp_damage >= self.rules.0.derived(&s.blueprint.sheet).hit_points)
    }

    /// A PC takes an attack's damage on its sheet (0x4b4390): hit points (at most all of them),
    /// poison, fatigue; the sheet is sent on ([`crate::sync_script_sheets`]).
    fn hurt_pc(&mut self, player: PlayerId, outcome: &AttackOutcome) {
        let rules = &self.rules.0;
        let Some(saved) = self.world.0.as_mut().and_then(|w| w.players.get_mut(&player)) else { return };
        let sheet = &mut saved.blueprint.sheet;
        let max = rules.derived(sheet).hit_points;
        sheet.hp_damage = (sheet.hp_damage + outcome.hp.max(0)).min(max);
        sheet.poison += outcome.poison.max(0);
        sheet.fatigue_damage += outcome.fatigue.max(0);
        self.sheets.0.insert(player);
    }

    fn tell(&mut self) {
        let delta = match &self.combat.0 {
            Some(e) => e.delta(),
            None => LedgerDelta::Combat { order: vec![], turn: None, ap: 0 },
        };
        crate::ground::broadcast(&self.session, &mut self.outbox, delta);
    }
}

/// Who fights around `centre` (0x4b71e0 / 0x4b73a0): the PCs within the radius first, then the
/// fighting NPCs, each in the order given; `first` leads.
fn gather(all: &[Seen], centre: TileLoc, first: Option<NetId>) -> Vec<NetId> {
    let near = |c: &&Seen| crate::within_radius(c.at, centre, ENGAGE_RADIUS);
    let mut order: Vec<NetId> = first.into_iter().collect();
    order.extend(all.iter().filter(near).filter(|c| c.player.is_some()).map(|c| c.net_id).filter(|n| Some(*n) != first));
    order.extend(all.iter().filter(near).filter(|c| c.npc.is_some() && c.fighting).map(|c| c.net_id).filter(|n| Some(*n) != first));
    order
}

/// Runs the fight: attacks and turn ends asked for, walks ending in attacks, NPC turns (passed
/// for now), points running out, rounds and the end. See the module docs.
pub fn run_combat(mut commands: Commands, mut w: CombatWorld, critters: Query<CombatantQuery>) {
    let all = seen(&critters);
    let find = |id: NetId| all.iter().find(|c| c.net_id == id).cloned();
    let mut changed = false;
    // NPCs made to fight this tick (their `Fighting` lands after the system).
    let mut joined: Vec<NetId> = Vec::new();
    // Fights started by dialogue or scripts: the NPC fights its target (0x4a9650).
    for (npc, target) in std::mem::take(&mut w.starts.0) {
        let (Some(attacker), Some(victim)) = (find(npc), find(target)) else { continue };
        let Some(source) = attacker.npc else { continue };
        if w.map.is_dead(source.sector, source.index) {
            continue;
        }
        commands.entity(attacker.entity).insert(Fighting { target });
        joined.push(npc);
        if !attacker.fighting && let Some(player) = victim.player {
            w.events.0.push((source.object(), EVENT_ENTER_COMBAT, player));
        }
        match w.combat.0.as_mut() {
            None => {
                let mut order = gather(&all, victim.at, None);
                for id in [target, npc] {
                    if !order.contains(&id) {
                        order.push(id);
                    }
                }
                let first = order.first().and_then(|id| find(*id));
                let ap = first.map_or(MIN_AP, |c| w.turn_ap(&c));
                w.combat.0 = Some(Encounter { order, turn: 0, ap, round: 1, fresh: true });
            }
            Some(e) => {
                if !e.order.contains(&npc) {
                    e.order.push(npc);
                }
            }
        }
        changed = true;
    }
    // Attacks asked for: the first starts the fight (0x4b3f20 → 0x4b71e0).
    for (player, target, _seq) in std::mem::take(&mut w.requests.attacks) {
        let (Some(pc), Some(victim)) = (all.iter().find(|c| c.player == Some(player)).cloned(), find(target)) else { continue };
        let Some(source) = victim.npc else { continue };
        if w.map.is_dead(source.sector, source.index) {
            continue;
        }
        if !victim.fighting {
            commands.entity(victim.entity).insert(Fighting { target: pc.net_id });
            joined.push(target);
            w.events.0.push((source.object(), EVENT_ENTER_COMBAT, player));
        }
        if w.combat.0.is_none() {
            let mut order = gather(&all, pc.at, Some(pc.net_id));
            if !order.contains(&target) {
                order.push(target);
            }
            let ap = w.turn_ap(&pc);
            w.combat.0 = Some(Encounter { order, turn: 0, ap, round: 1, fresh: true });
            changed = true;
        }
        let Some(e) = w.combat.0.as_mut() else { continue };
        if !e.order.contains(&target) {
            e.order.push(target);
            changed = true;
        }
        if e.current() == Some(pc.net_id) {
            changed |= attack_or_approach(&mut commands, &mut w, &pc, &victim);
        }
    }
    for player in std::mem::take(&mut w.requests.end_turn) {
        let Some(pc) = all.iter().find(|c| c.player == Some(player)) else { continue };
        if w.combat.0.as_ref().and_then(Encounter::current) == Some(pc.net_id) {
            next_turn(&mut w, &all);
            changed = true;
        }
    }
    // A walk to the target over: the attack.
    if let Some(current) = w.combat.0.as_ref().and_then(Encounter::current)
        && let Some(c) = find(current)
        && let (Some(p), false) = (c.pending, c.walking)
    {
        commands.entity(c.entity).remove::<PendingAttack>();
        if let Some(victim) = find(p.target) {
            changed |= attack_or_approach(&mut commands, &mut w, &c, &victim);
        }
    }
    // An NPC's turn: flee or fight; a PC out of points, down or gone ends its turn.
    if let Some(e) = w.combat.0.clone() {
        let current = e.current().and_then(find);
        let done = match &current {
            None => true,
            Some(c) if c.walking || c.pending.is_some() => false,
            Some(c) if c.npc.is_some_and(|n| w.map.is_dead(n.sector, n.index)) => true,
            Some(c) if c.npc.is_some() => {
                let acted = npc_turn(&mut commands, &mut w, c, &all, e.fresh);
                if let Some(e) = w.combat.0.as_mut() {
                    e.fresh = false;
                }
                changed |= acted;
                !acted
            }
            Some(c) => e.ap <= 0 || w.pc_down(c),
        };
        if done {
            next_turn(&mut w, &all);
            changed = true;
        }
    }
    // No NPC left fighting, or no PC standing: the fight is over at once (0x4b7580 finds no
    // active critter).
    if let Some(e) = &w.combat.0 {
        let fights = e.order.iter().filter_map(|id| find(*id)).any(|c| c.npc.is_some_and(|n| !w.map.is_dead(n.sector, n.index)) && (c.fighting || joined.contains(&c.net_id)));
        let stands = e.order.iter().filter_map(|id| find(*id)).any(|c| c.player.is_some() && !w.pc_down(&c));
        if !fights || !stands {
            w.combat.0 = None;
            changed = true;
        }
    }
    if changed {
        w.tell();
    }
}

/// The next critter's turn (0x4b7690); past the last, a new round gathered again (0x4b73a0),
/// or the end when no NPC fights (0x4b7330).
fn next_turn(w: &mut CombatWorld, all: &[Seen]) {
    let Some((turn, len, centre)) = w.combat.0.as_mut().map(|e| {
        e.turn += 1;
        let centre = e.order.iter().find_map(|id| all.iter().find(|c| c.net_id == *id && c.player.is_some())).map(|c| c.at);
        (e.turn, e.order.len(), centre)
    }) else {
        return;
    };
    if turn >= len {
        let alive: Vec<Seen> = all.iter().filter(|c| c.npc.is_none_or(|n| !w.map.is_dead(n.sector, n.index))).cloned().collect();
        let order = centre.map(|at| gather(&alive, at, None)).unwrap_or_default();
        let fights = order.iter().any(|id| alive.iter().any(|c| c.net_id == *id && c.npc.is_some()));
        let stands = order.iter().any(|id| alive.iter().any(|c| c.net_id == *id && c.player.is_some() && !w.pc_down(c)));
        if !fights || !stands {
            w.combat.0 = None;
            return;
        }
        if let Some(e) = w.combat.0.as_mut() {
            e.order = order;
            e.turn = 0;
            e.round += 1;
        }
    }
    let next = w.combat.0.as_ref().and_then(Encounter::current).and_then(|id| all.iter().find(|c| c.net_id == id)).cloned();
    let ap = next.map_or(0, |c| w.turn_ap(&c));
    if let Some(e) = w.combat.0.as_mut() {
        e.ap = ap;
        e.fresh = true;
    }
}

/// The attacker strikes if the target is in reach and the points last; a melee attacker out of
/// reach walks next to it first, as far as its points go (0x4b78d0). True if something changed.
fn attack_or_approach(commands: &mut Commands, w: &mut CombatWorld, attacker: &Seen, victim: &Seen) -> bool {
    let fighter = w.fighter(attacker);
    let cost = fighter.attack_cost();
    let ap = w.combat.0.as_ref().map_or(0, |e| e.ap);
    if reach_between(attacker.at, victim.at) <= fighter.reach() {
        // A PC with points left may act past them (0x4b7790); its turn ends after.
        if ap < cost && !(ap > 0 && attacker.player.is_some()) {
            return false;
        }
        if let Some(e) = w.combat.0.as_mut() {
            e.ap = (e.ap - cost).max(0);
        }
        strike(commands, w, attacker, victim, &fighter);
        return true;
    }
    if fighter.reach() > 1 {
        return false;
    }
    // A path to a tile next to the target, as many steps as the points pay for.
    let goals = neighbours(victim.at);
    let path = goals.into_iter().filter_map(|g| crate::find_path(&*w.map, attacker.at, g).ok()).min_by_key(Vec::len);
    let Some(mut path) = path else { return false };
    let steps = (ap / STEP_AP).max(0) as usize;
    path.truncate(steps);
    if path.is_empty() {
        return false;
    }
    if let Some(e) = w.combat.0.as_mut() {
        e.ap -= path.len() as i32 * STEP_AP;
    }
    commands.entity(attacker.entity).insert((crate::Walking { steps: path.into(), progress: 0.0 }, PendingAttack { target: victim.net_id }));
    true
}

/// `rules/ai_params.mes`: 17 numbers a row, the row by `npc_ai_data` (0x4aaa60).
#[derive(Resource, Debug, Default)]
pub struct AiParams(pub Option<Vec<[i32; 17]>>);

/// The AI rows of `mes`.
pub fn parse_ai_params(mes: &arcanum_formats::mes::MesFile) -> Vec<[i32; 17]> {
    let mut rows = Vec::new();
    for n in 0.. {
        let Some(line) = mes.get(n) else { break };
        let mut row = [0; 17];
        for (r, v) in row.iter_mut().zip(line.split_whitespace()) {
            *r = v.parse().unwrap_or(0);
        }
        rows.push(row);
    }
    rows
}

/// `critter_flags`: never flees (0x4ab2f0).
pub const OCF_NO_FLEE: i32 = 0x0800_0000;
/// How far a fleeing NPC runs in one turn at most, in tiles (builder's call).
pub const FLEE_TILES: u32 = 8;

/// Whether an NPC at `hp_percent` flees (0x4ab2f0): at or below its row's percent (column 0),
/// unless it never flees. The original also weighs the target's hit points and the distances
/// (columns 1-3); not built.
pub fn should_flee(hp_percent: i32, row: &[i32; 17], critter_flags: i32) -> bool {
    critter_flags & OCF_NO_FLEE == 0 && hp_percent <= row[0]
}

/// A fighting NPC's turn: at its start, flee if hurt enough (it walks away and stops fighting);
/// else attack its target, or a standing PC of the fight when that one is gone, walking next to
/// it first. False when it has nothing more to do.
fn npc_turn(commands: &mut Commands, w: &mut CombatWorld, npc: &Seen, all: &[Seen], fresh: bool) -> bool {
    let Some(source) = npc.npc else { return false };
    if !npc.fighting {
        return false;
    }
    if fresh {
        if w.ai.0.is_none() {
            let vfs = w.vfs.as_ref().map(|v| v.0.clone());
            let guard = vfs.as_ref().and_then(|v| v.read().ok());
            let mes = guard.as_deref().and_then(|v| v.read(AI_PARAMS).ok()).and_then(|d| arcanum_formats::mes::MesFile::parse(&d).ok()).unwrap_or_default();
            w.ai.0 = Some(parse_ai_params(&mes));
        }
        let o = w.map.mobiles.get(&source.sector).and_then(|m| m.get(source.index as usize));
        let row = o.and_then(|o| w.ai.0.as_ref()?.get(w.map.prototypes.int(o, "npc_ai_data").unwrap_or(0).max(0) as usize).copied());
        let flags = o.and_then(|o| w.map.prototypes.int(o, "critter_flags")).unwrap_or(0);
        let percent = w.map.npc_hp(source.sector, source.index).map_or(100, |(left, max)| if max > 0 { left * 100 / max } else { 100 });
        if let Some(row) = row
            && should_flee(percent, &row, flags)
        {
            let from_critter = npc.target.and_then(|t| all.iter().find(|c| c.net_id == t));
            let from = from_critter.map_or(npc.at, |c| c.at);
            commands.entity(npc.entity).remove::<Fighting>();
            if let Some(player) = from_critter.and_then(|c| c.player) {
                w.events.0.push((source.object(), EVENT_EXIT_COMBAT, player));
            }
            let away = flee_goal(npc.at, from, FLEE_TILES);
            if let Ok(mut path) = crate::find_path(&*w.map, npc.at, away) {
                let ap = w.combat.0.as_ref().map_or(0, |e| e.ap);
                path.truncate((ap / STEP_AP).max(0) as usize);
                if !path.is_empty() {
                    commands.entity(npc.entity).insert(crate::Walking { steps: path.into(), progress: 0.0 });
                }
            }
            if let Some(e) = w.combat.0.as_mut() {
                e.ap = 0;
            }
            return false;
        }
    }
    let in_fight = |c: &&Seen| w.combat.0.as_ref().is_some_and(|e| e.order.contains(&c.net_id));
    let target = npc.target.and_then(|t| all.iter().find(|c| c.net_id == t)).filter(|c| c.player.is_none() || !w.pc_down(c)).cloned();
    let target = target.or_else(|| all.iter().filter(in_fight).filter(|c| c.player.is_some() && !w.pc_down(c)).min_by_key(|c| crate::tile_distance(c.at, npc.at)).cloned());
    let Some(target) = target else { return false };
    if npc.target != Some(target.net_id) {
        commands.entity(npc.entity).insert(Fighting { target: target.net_id });
    }
    attack_or_approach(commands, w, npc, &target)
}

/// `rules/ai_params.mes`.
pub const AI_PARAMS: &str = "rules/ai_params.mes";

/// `npc_flags`: kill on sight (0x4ae120), and its "made peaceful" bit.
pub const ONF_KOS: i32 = 0x100;
pub const ONF_KOS_OVERRIDE: i32 = 0x800;
/// How often NPCs look for foes, in ticks (builder's call: the original on each heartbeat).
pub const NOTICE_TICKS: u64 = 15;

/// Whether an NPC is hostile to a PC (0x4ae120): kill on sight; or its reaction at or below its
/// row's column 10; or their alignments at least column 11 apart.
pub fn hostile(npc_flags: i32, reaction: i32, alignment_gap: i32, row: &[i32; 17]) -> bool {
    (npc_flags & ONF_KOS != 0 && npc_flags & ONF_KOS_OVERRIDE == 0) || reaction <= row[10] || alignment_gap >= row[11]
}

/// How far an NPC of Perception `pe` notices a PC (0x4ab460 / 0x4af260: `clamp(PE, 1, 10)`
/// tiles, Perception first cut by a prowler's value), or hears it (0x4af470).
pub fn notice_reach(pe: i32, prowler: Option<i32>) -> u32 {
    let seen = pe - prowler.map_or(0, |p| pe * p.clamp(0, 100) / 100);
    (seen.clamp(1, 10) as u32).max(crate::prowl::hearing_reach(pe, prowler) as u32)
}

type FoeQuery<'a> = (&'a NetId, &'a crate::Position, &'a crate::NpcSource);
type QuarryQuery<'a> = (&'a NetId, &'a crate::Position, &'a crate::PlayerControlled, &'a crate::Sheet, Has<crate::Prowling>);

/// NPCs that are not fighting look for PCs they are hostile to within reach (0x4aa0d0 →
/// 0x4ab460 → 0x4ae120); one found starts a fight ([`CombatStarts`]). Followers, the dead and
/// those talking keep still. Sight lines and facing are not counted.
#[allow(clippy::too_many_arguments)]
pub fn notice_foes(
    tick: Res<crate::SimTick>,
    map: Res<crate::LoadedMap>,
    world: Res<crate::WorldState>,
    rules: Res<crate::SimRules>,
    (mut ai, mut reputations, vfs): (ResMut<AiParams>, ResMut<crate::ReputationTable>, Option<Res<crate::SimVfs>>),
    mut starts: ResMut<CombatStarts>,
    npcs: Query<FoeQuery, (Without<Fighting>, Without<crate::follow::Following>)>,
    pcs: Query<QuarryQuery>,
    talking: Query<&crate::Conversation>,
) {
    if !tick.0.is_multiple_of(NOTICE_TICKS) || pcs.is_empty() {
        return;
    }
    let Some(save) = world.0.as_ref() else { return };
    let guard = vfs.as_ref().and_then(|v| v.0.read().ok());
    if ai.0.is_none() {
        let mes = guard.as_deref().and_then(|v| v.read(AI_PARAMS).ok()).and_then(|d| arcanum_formats::mes::MesFile::parse(&d).ok()).unwrap_or_default();
        ai.0 = Some(parse_ai_params(&mes));
    }
    let reputations = &*reputations.0.get_or_insert_with(|| guard.as_deref().and_then(|v| v.read(crate::GAME_REP).ok()).and_then(|d| arcanum_formats::mes::MesFile::parse(&d).ok()).map(|m| crate::ReputationEffects::parse(&m)).unwrap_or_default());
    let rows = ai.0.as_deref().unwrap_or_default();
    let busy: Vec<NetId> = talking.iter().map(|c| c.npc).collect();
    for (&npc, at, source) in &npcs {
        if map.is_dead(source.sector, source.index) || busy.contains(&npc) {
            continue;
        }
        let Some(o) = map.mobiles.get(&source.sector).and_then(|m| m.get(source.index as usize)) else { continue };
        let Some(row) = rows.get(map.prototypes.int(o, "npc_ai_data").unwrap_or(0).max(0) as usize) else { continue };
        let flags = map.prototypes.int(o, "npc_flags").unwrap_or(0);
        let pe = npc_stat(o, &map.prototypes, 5);
        let reaction = crate::NpcReaction::of(o, &map.prototypes);
        let key = crate::met_key(source.object());
        let alignment = npc_stat(o, &map.prototypes, 0x13);
        for (&pc, pc_at, player, sheet, prowling) in &pcs {
            let Some(saved) = save.players.get(&player.0) else { continue };
            let prowler = prowling.then(|| crate::prowl::prowl_value(rules.0.skill_level(&sheet.0, crate::SKILL_PROWLING)));
            if crate::tile_distance(at.0, pc_at.0) > notice_reach(pe, prowler) {
                continue;
            }
            let towards = crate::reaction_to(&saved.script_state, &key, &reaction, &rules.0, &sheet.0, reputations);
            let gap = (alignment - sheet.0.alignment).abs();
            if hostile(flags, towards, gap, row) {
                starts.0.push((npc, pc));
                break;
            }
        }
    }
}

/// A tile up to `tiles` away from `from`, straight on past `at`.
fn flee_goal(at: TileLoc, from: TileLoc, tiles: u32) -> TileLoc {
    let dx = i64::from(at.x) - i64::from(from.x);
    let dy = i64::from(at.y) - i64::from(from.y);
    let (sx, sy) = (dx.signum(), dy.signum());
    let (sx, sy) = if (sx, sy) == (0, 0) { (1, 0) } else { (sx, sy) };
    let x = (i64::from(at.x) + sx * i64::from(tiles)).max(0) as u32;
    let y = (i64::from(at.y) + sy * i64::from(tiles)).max(0) as u32;
    TileLoc { x, y }
}

/// The eight tiles round `at`.
pub fn neighbours(at: TileLoc) -> Vec<TileLoc> {
    let mut out = Vec::new();
    for dy in -1i64..=1 {
        for dx in -1i64..=1 {
            if (dx, dy) != (0, 0) {
                let (x, y) = (i64::from(at.x) + dx, i64::from(at.y) + dy);
                if let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) {
                    out.push(TileLoc { x, y });
                }
            }
        }
    }
    out
}

/// One attack and what it does: an NPC victim fights back; its damage, death and the
/// experience; everyone is told.
fn strike(commands: &mut Commands, w: &mut CombatWorld, attacker: &Seen, victim: &Seen, fighter: &Fighter) {
    let defender = w.fighter(victim);
    let outcome = resolve(fighter, &defender, &mut w.rng);
    crate::ground::broadcast(&w.session, &mut w.outbox, LedgerDelta::Attacked { attacker: attacker.net_id, target: victim.net_id, outcome });
    if let Some(player) = victim.player {
        if outcome.hit {
            w.hurt_pc(player, &outcome);
        }
        return;
    }
    let Some(source) = victim.npc else { return };
    commands.entity(victim.entity).insert(Fighting { target: attacker.net_id });
    if !outcome.hit || outcome.hp <= 0 {
        return;
    }
    let Some((left, max)) = w.map.npc_hp(source.sector, source.index) else { return };
    let dealt = outcome.hp.min(left.max(0));
    w.map.hurt_npc(source.sector, source.index, outcome.hp);
    // Experience for the damage (0x4b4390) and the kill (0x45da20: a fifth of its worth).
    let (worth, pool) = w.map.mobiles.get(&source.sector).and_then(|m| m.get(source.index as usize)).map_or((0, 0), |o| (w.map.prototypes.int(o, "npc_experience_worth").unwrap_or(0), w.map.prototypes.int(o, "npc_experience_pool").unwrap_or(0)));
    let mut xp = 0;
    if attacker.player.is_some() {
        xp = hit_experience(dealt, max, worth, pool);
        if xp > 0 {
            w.map.spend_npc_pool(source.sector, source.index, xp);
        }
    }
    if left - outcome.hp < 1 {
        let vfs = w.vfs.as_ref().map(|v| v.0.clone());
        let guard = vfs.as_ref().and_then(|v| v.read().ok());
        w.map.kill_npc(guard.as_deref(), source.sector, source.index);
        commands.entity(victim.entity).remove::<Fighting>();
        w.toggled.0.push((source.sector, source.index));
        if let Some(player) = attacker.player {
            w.events.0.push((source.object(), EVENT_DYING, player));
        }
        if attacker.player.is_some() {
            xp += worth * 20 / 100;
        }
    }
    if xp > 0 {
        w.awards.0.push(xp);
    }
    crate::ground::save_change(&mut w.world, &w.map, source.sector);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chances_and_costs_follow_the_original() {
        assert_eq!((weapon_chance(SKILL_MELEE, 0), weapon_chance(SKILL_MELEE, 4), weapon_chance(SKILL_MELEE, 20)), (25, 45, 95));
        assert_eq!((weapon_chance(SKILL_DODGE, 5), weapon_chance(SKILL_THROWING, 2)), (10, 39));
        assert_eq!((attack_cost(0), attack_cost(9), attack_cost(21), attack_cost(30)), (8, 5, 2, 1));
        assert_eq!(Fighter::default().attack_cost(), 5, "bare hands: speed 10");
        assert_eq!(pc_unarmed(0), [(1, 5), (0, 0), (0, 0), (0, 0), (1, 5)]);
        assert_eq!(pc_unarmed(40)[0], (15, 45));
        assert_eq!(hit_experience(10, 20, 100, 100), 50);
        assert_eq!(hit_experience(10, 20, 100, 30), 30, "what is left");
    }

    #[test]
    fn hostility_and_reach_follow_the_ai_row() {
        let row = [0, 1000, 1000, 20, 6, 20, 1001, 1000, 2, 0, 0, 2001, 100, 50, 20, 1, 1];
        assert!(hostile(ONF_KOS, 60, 0, &row), "kill on sight");
        assert!(!hostile(ONF_KOS | ONF_KOS_OVERRIDE, 60, 0, &row), "made peaceful");
        assert!(hostile(0, 0, 0, &row) && !hostile(0, 1, 0, &row), "reaction at or below column 10");
        assert!(!hostile(0, 50, 1000, &row), "alignment: column 11 (2001) is out of reach");
        assert_eq!((notice_reach(8, None), notice_reach(20, None), notice_reach(8, Some(100))), (8, 10, 3));
    }

    #[test]
    fn npcs_flee_by_their_ai_row() {
        let rows = parse_ai_params(&arcanum_formats::mes::MesFile::parse(b"{0}{0 1000 1000 20 6 20 1001 1000 2 0 0 2001 100 50 20 1 1}{1}{50 20 3 20 20 20 20 20 2 0 0 2001 100 15 100 0 1}").unwrap());
        assert_eq!(rows.len(), 2);
        assert!(!should_flee(10, &rows[0], 0), "row 0 never flees");
        assert!(should_flee(50, &rows[1], 0) && !should_flee(51, &rows[1], 0));
        assert!(!should_flee(10, &rows[1], OCF_NO_FLEE));
        assert_eq!(flee_goal(TileLoc { x: 10, y: 10 }, TileLoc { x: 9, y: 10 }, 8), TileLoc { x: 18, y: 10 });
    }

    #[test]
    fn attacks_hit_miss_and_resistance_cuts_damage() {
        let sword = Weapon { skill: SKILL_MELEE, damage: [(4, 4), (0, 0), (0, 0), (0, 0), (0, 0)], ..Default::default() };
        let master = Fighter { levels: vec![(SKILL_MELEE, 20)], weapon: Some(sword), ..Default::default() };
        let armoured = Fighter { resistances: [50, 0, 0, 0, 0], ..Default::default() };
        let mut rng = HostRng::default();
        let mut hits = 0;
        for _ in 0..200 {
            let out = resolve(&master, &armoured, &mut rng);
            if out.hit {
                hits += 1;
                assert!(out.hp == 2 || out.critical, "4 less half: {out:?}");
            }
        }
        assert!(hits > 160, "95 %: {hits}");
        let novice = Fighter { levels: vec![(SKILL_MELEE, 0)], ..Default::default() };
        let misses = (0..200).filter(|_| !resolve(&novice, &armoured, &mut rng).hit).count();
        assert!(misses > 100, "25 %: {misses}");
    }
}
