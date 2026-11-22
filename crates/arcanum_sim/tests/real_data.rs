//! Runs against the real game data; skipped when `assets/` is absent.

use std::path::PathBuf;

use arcanum_formats::map::TileLoc;
use arcanum_sim::StartMap;
use arcanum_vfs::Vfs;

/// PATCH0, then Arcanum.dat, then the game folder's archives (as in `arcanum_formats`' tests).
fn vfs() -> Option<Vfs> {
    let game = std::env::var_os("ARCANUM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/Arcanum"));
    let modules = game.join("modules");
    if !modules.join("Arcanum.dat").is_file() {
        eprintln!("skipping: game data not found");
        return None;
    }
    let mut vfs = Vfs::new();
    let patch = modules.join("Arcanum.PATCH0");
    if patch.is_file() {
        vfs.mount_archive(patch).unwrap();
    }
    vfs.mount_archive(modules.join("Arcanum.dat")).unwrap();
    vfs.mount_archives_in(&game).unwrap();
    // The loose prototypes and pre-made characters, as the game mounts them.
    vfs.mount_dir(game.join("data"));
    Some(vfs)
}

#[test]
fn new_games_start_where_the_original_does() {
    let Some(vfs) = vfs() else { return };
    let start = StartMap::load(&vfs).unwrap();
    // The clock from rules/gameinit.mes: 1885, 15:00 ("Currently 3pm, WAS 10am").
    let clock = arcanum_sim::GameStart { year: 1885, hour: 15 };
    assert_eq!(start, StartMap { map: "Arcanum1-024-fixed".into(), position: TileLoc { x: 92958, y: 82592 }, clock });
}

#[test]
fn a_new_game_boots_on_the_start_map() {
    use std::sync::{Arc, RwLock};

    use arcanum_rules::{CharacterBlueprint, CharacterSheet, RulesData};
    use arcanum_sim::{LoadedMap, PlayerId, Position, SimApp, SimVfs, WorldState};

    let Some(vfs) = vfs() else { return };
    let start = StartMap::load(&vfs).unwrap();
    let mut world = WorldState::default();
    let sheet = CharacterSheet { name: "Virgil".into(), ..Default::default() };
    world.add_player(Some(&start), PlayerId(1), CharacterBlueprint { sheet, gold: 0, items: vec![], prototype: None });

    let mut sim = SimApp::new(RulesData::default());
    sim.use_manual_clock();
    sim.app.insert_resource(SimVfs(Arc::new(RwLock::new(vfs))));
    sim.boot(world.0.unwrap());
    sim.update();
    sim.update();

    let map = sim.app.world().resource::<LoadedMap>();
    assert!(map.errors.is_empty(), "{:?}", map.errors);
    assert!(map.properties.is_some());
    assert!(map.sectors.contains_key(&start.position.sector()), "the PC's sector");
    assert_eq!(map.sectors.len() + map.absent.len(), 9);
    eprintln!("loaded {} sectors around the start, {} absent", map.sectors.len(), map.absent.len());
    let mobiles: usize = map.mobiles.values().map(Vec::len).sum();
    let world = sim.app.world_mut();
    let pcs: Vec<TileLoc> = world.query_filtered::<&Position, bevy_ecs::query::With<arcanum_sim::PlayerControlled>>().iter(world).map(|p| p.0).collect();
    assert_eq!(pcs, vec![start.position]);
    let npcs: Vec<(TileLoc, u32)> = world.query::<(&Position, &arcanum_sim::Npc)>().iter(world).map(|(p, n)| (p.0, n.prototype)).collect();
    eprintln!("{mobiles} mobile objects on the map; {} NPCs round the start: {npcs:?}", npcs.len());
    let wanderers: Vec<TileLoc> = world.query::<(&Position, &arcanum_sim::Standpoints)>().iter(world).filter(|(_, s)| s.wanders).map(|(p, _)| p.0).collect();
    eprintln!("{} of them wander: {wanderers:?}", wanderers.len());
    // Virgil stands next to the PC, and the dying gnome two tiles down (start-map .mob files).
    assert!(npcs.iter().any(|&(p, _)| p == TileLoc { x: 92959, y: 82592 }), "{npcs:?}");
    assert!(npcs.iter().any(|&(p, _)| p == TileLoc { x: 92959, y: 82594 }), "{npcs:?}");
}

#[test]
fn paths_on_the_start_map_pass_the_hosts_check() {
    use std::sync::{Arc, RwLock};

    use arcanum_rules::{CharacterBlueprint, CharacterSheet, RulesData};
    use arcanum_sim::{LoadedMap, PlayerId, SimApp, SimVfs, Walkable, WorldState, check_path, find_path};

    let Some(vfs) = vfs() else { return };
    let start = StartMap::load(&vfs).unwrap();
    let mut world = WorldState::default();
    let sheet = CharacterSheet { name: "Virgil".into(), ..Default::default() };
    world.add_player(Some(&start), PlayerId(1), CharacterBlueprint { sheet, gold: 0, items: vec![], prototype: None });
    let mut sim = SimApp::new(RulesData::default());
    sim.use_manual_clock();
    sim.app.insert_resource(SimVfs(Arc::new(RwLock::new(vfs))));
    sim.boot(world.0.unwrap());
    sim.update();
    sim.update();
    let map = sim.app.world().resource::<LoadedMap>();

    assert!(map.walkable(start.position), "the start tile");
    // Every walkable tile within 20 of the start that A* reaches gets a path the host accepts.
    let (mut found, mut blocked) = (0, 0);
    for dy in -20i32..=20 {
        for dx in -20i32..=20 {
            let goal = TileLoc { x: start.position.x.wrapping_add_signed(dx), y: start.position.y.wrapping_add_signed(dy) };
            if goal == start.position {
                continue;
            }
            if !map.walkable(goal) {
                blocked += 1;
                continue;
            }
            if let Ok(path) = find_path(map, start.position, goal) {
                found += 1;
                assert_eq!(check_path(map, start.position, &path).as_ref(), Ok(&path), "to {goal:?}");
            }
        }
    }
    let edges: usize = map.blocking.edges.values().map(|e| e.count_ones() as usize).sum();
    eprintln!(
        "{found} tiles reached, {blocked} blocked, of 41 x 41 around the start; loaded: {} prototypes, {} tiles and {edges} edges blocked by objects, errors {:?}",
        map.prototypes.len(),
        map.blocking.tiles.len(),
        map.errors
    );
    assert!(map.errors.is_empty(), "{:?}", map.errors);
    assert!(map.prototypes.len() > 1000);
    assert!(found > 100, "{found}");
    assert!(blocked > 0, "the wreck and the scenery round the start block");
}

/// In a Shrouded Hills house, walls block by edge: no path crosses one, and the way in and out
/// is through the doors.
#[test]
fn house_walls_block_and_doors_let_through() {
    use arcanum_formats::map::{MAP_LIST, parse_map_list, start_map};
    use arcanum_formats::mes::MesFile;
    use arcanum_sim::{LoadedMap, Walkable, find_path, step_allowed};

    let Some(vfs) = vfs() else { return };
    let list = parse_map_list(&MesFile::parse(&vfs.read(MAP_LIST).unwrap()).unwrap()).unwrap();
    let name = start_map(&list).unwrap().name.clone();
    let mut map = LoadedMap::default();
    map.open(Some(&vfs), &name);
    let inside = TileLoc { x: 90330, y: 84122 };
    map.load_around(Some(&vfs), inside);
    let edges: usize = map.blocking.edges.values().map(|e| e.count_ones() as usize).sum();
    let doors: usize = map.blocking.diagonal_edges.values().map(|e| e.count_ones() as usize).sum();
    eprintln!("{edges} wall edges, {doors} door-opening edges, {} blocked tiles", map.blocking.tiles.len());
    assert!(edges > 100, "{edges}");
    // Every wall edge stops a step across it, both ways.
    let mut checked = 0;
    for (&at, &mask) in &map.blocking.edges {
        for dir in (1..8).step_by(2) {
            if mask >> dir & 1 == 0 {
                continue;
            }
            let (dx, dy) = arcanum_sim::DIRECTIONS[dir];
            let next = TileLoc { x: at.x.wrapping_add_signed(dx), y: at.y.wrapping_add_signed(dy) };
            assert!(!step_allowed(&map, at, next) && !step_allowed(&map, next, at), "{at:?} dir {dir}");
            checked += 1;
        }
    }
    assert_eq!(checked, edges);
    // From inside the house to well outside: there is a way (through a door).
    // The tile itself has furniture on it: start from a free one next to it.
    let inside = (0..9u32).map(|i| TileLoc { x: inside.x + i % 3 - 1, y: inside.y + i / 3 - 1 }).find(|&t| map.walkable(t)).expect("a free tile");
    let outside = TileLoc { x: inside.x + 12, y: inside.y + 12 };
    let path = find_path(&map, inside, outside);
    eprintln!("inside to outside: {:?} steps", path.as_ref().map(Vec::len));
    assert!(path.is_ok(), "{path:?}");
}

#[test]
fn a_chest_on_the_start_map_opens_with_its_stored_contents() {
    use std::sync::{Arc, RwLock};

    use arcanum_formats::object::TYPE_CONTAINER;
    use arcanum_rules::{CharacterBlueprint, CharacterSheet, RulesData};
    use arcanum_sim::{GroundItemId, LedgerDelta, LoadedMap, PlayerId, PlayerIntent, SimApp, SimVfs, WorldState, object_location, open_check};

    let Some(vfs) = vfs() else { return };
    let vfs = Arc::new(RwLock::new(vfs));
    // The nearest unlocked container with a stored list in the sectors round the start.
    let start = StartMap::load(&vfs.read().unwrap()).unwrap();
    let boot = |at: TileLoc| {
        let mut world = WorldState::default();
        let sheet = CharacterSheet { name: "Virgil".into(), ..Default::default() };
        let start = StartMap { position: at, ..start.clone() };
        world.add_player(Some(&start), PlayerId(1), CharacterBlueprint { sheet, gold: 0, items: vec![], prototype: None });
        let mut sim = SimApp::new(RulesData::default());
        sim.use_manual_clock();
        sim.app.insert_resource(SimVfs(vfs.clone()));
        sim.boot(world.0.unwrap());
        sim.update();
        sim.update();
        sim
    };
    let sim = boot(start.position);
    let map = sim.app.world().resource::<LoadedMap>();
    let mut chests: Vec<(u64, GroundItemId, TileLoc)> = Vec::new();
    for (&sector, objects) in &map.mobiles {
        for (index, object) in objects.iter().enumerate() {
            let id = GroundItemId::Container { sector, index: index as u32 };
            let Some(at) = object_location(object).filter(|_| object.object_type == TYPE_CONTAINER) else { continue };
            let flags = map.prototypes.int(object, "container_flags").unwrap_or(0);
            if open_check(flags, map.lock_day, false).is_ok() && map.pile(id).is_some_and(|p| !p.inventory.items.is_empty()) {
                let d = (at.x.abs_diff(start.position.x) as u64).pow(2) + (at.y.abs_diff(start.position.y) as u64).pow(2);
                chests.push((d, id, at));
            }
        }
    }
    chests.sort_by_key(|c| c.0);
    let &(_, _, at) = chests.first().expect("a chest near the start");
    let expected = map.pile(chests[0].1).unwrap().inventory;
    eprintln!("{} openable chests with contents round the start; nearest at {at:?}: {:?}", chests.len(), expected.items);
    // A PC standing on it opens it and sees the same contents.
    let mut sim = boot(at);
    sim.take_deltas();
    let map = sim.app.world().resource::<LoadedMap>();
    let id = map.mobiles[&at.sector()].iter().position(|o| object_location(o) == Some(at) && o.object_type == TYPE_CONTAINER).map(|i| GroundItemId::Container { sector: at.sector(), index: i as u32 }).unwrap();
    sim.receive([(PlayerId(1), PlayerIntent::PickUp { item: id, seq: 0 })]);
    sim.update();
    let deltas = sim.take_deltas();
    assert!(deltas.iter().any(|(_, d)| *d == LedgerDelta::PileOpened { pile: id }), "{deltas:?}");
    assert_eq!(sim.app.world().resource::<LoadedMap>().pile(id).unwrap().inventory, expected);
}

#[test]
fn a_key_from_a_chest_on_the_start_map_opens_its_locked_chest() {
    use std::sync::{Arc, RwLock};

    use arcanum_formats::object::TYPE_CONTAINER;
    use arcanum_rules::{CharacterBlueprint, CharacterSheet, RulesData};
    use arcanum_sim::{GroundItemId, Inventory, InventoryRequest, LedgerDelta, LoadedMap, PlayerControlled, PlayerId, PlayerIntent, Position, SimApp, SimVfs, WorldState, key_matches, object_location, open_check};

    let Some(vfs) = vfs() else { return };
    let vfs = Arc::new(RwLock::new(vfs));
    let start = StartMap::load(&vfs.read().unwrap()).unwrap();
    let mut world = WorldState::default();
    let sheet = CharacterSheet { name: "Virgil".into(), ..Default::default() };
    world.add_player(Some(&start), PlayerId(1), CharacterBlueprint { sheet, gold: 0, items: vec![], prototype: None });
    let mut sim = SimApp::new(RulesData::default());
    sim.use_manual_clock();
    sim.app.insert_resource(SimVfs(vfs.clone()));
    sim.boot(world.0.unwrap());
    sim.update();
    sim.update();
    sim.take_deltas();
    // A locked chest round the start whose key lies in an openable one.
    let map = sim.app.world().resource::<LoadedMap>();
    let mut chests: Vec<(GroundItemId, TileLoc, i32, i32)> = Vec::new();
    for (&sector, objects) in &map.mobiles {
        for (index, object) in objects.iter().enumerate() {
            let Some(at) = object_location(object).filter(|_| object.object_type == TYPE_CONTAINER) else { continue };
            let flags = map.prototypes.int(object, "container_flags").unwrap_or(0);
            let lock = map.prototypes.int(object, "container_key_id").unwrap_or(0);
            chests.push((GroundItemId::Container { sector, index: index as u32 }, at, flags, lock));
        }
    }
    let pair = chests.iter().filter(|c| open_check(c.2, map.lock_day, false).is_err() && open_check(c.2, map.lock_day, true).is_ok()).find_map(|locked| {
        chests.iter().filter(|c| open_check(c.2, map.lock_day, false).is_ok()).find_map(|holder| {
            let items = map.pile(holder.0)?.inventory.items;
            let key = items.iter().find(|i| i.keys.iter().any(|&k| key_matches(locked.3, k)))?;
            Some((*locked, *holder, key.location))
        })
    });
    let ((locked, locked_at, _, lock), (holder, holder_at, ..), from) = pair.expect("a locked chest with its key in another");
    eprintln!("lock {lock} at {locked_at:?}, its key in the chest at {holder_at:?}");
    let place = |sim: &mut SimApp, at: TileLoc| {
        let world = sim.app.world_mut();
        world.query_filtered::<&mut Position, bevy_ecs::query::With<PlayerControlled>>().single_mut(world).unwrap().0 = at;
    };
    let open = |sim: &mut SimApp, id: GroundItemId| {
        sim.receive([(PlayerId(1), PlayerIntent::PickUp { item: id, seq: 0 })]);
        sim.update();
        sim.take_deltas().into_iter().any(|(_, d)| d == LedgerDelta::PileOpened { pile: id })
    };
    place(&mut sim, locked_at);
    assert!(!open(&mut sim, locked), "locked without the key");
    place(&mut sim, holder_at);
    assert!(open(&mut sim, holder));
    sim.receive([
        (PlayerId(1), PlayerIntent::Inventory { seq: 1, request: InventoryRequest::Take { from, to: None } }),
        (PlayerId(1), PlayerIntent::Inventory { seq: 2, request: InventoryRequest::ClosePile }),
    ]);
    sim.update();
    let world = sim.app.world_mut();
    let carried = world.query_filtered::<&Inventory, bevy_ecs::query::With<PlayerControlled>>().single(world).unwrap().clone();
    assert!(carried.has_key(lock), "{carried:?}");
    place(&mut sim, locked_at);
    assert!(open(&mut sim, locked), "the key opens it");
}

#[test]
fn a_skilled_pc_picks_a_locked_chest_on_the_start_map_with_a_lockpick() {
    use std::sync::{Arc, RwLock};

    use arcanum_formats::object::TYPE_CONTAINER;
    use arcanum_rules::{CharacterBlueprint, CharacterSheet, RulesData};
    use arcanum_sim::{CarriedItem, GroundItemId, Inventory, LedgerDelta, LoadedMap, LockTarget, PlayerControlled, PlayerId, PlayerIntent, Position, SimApp, SimVfs, Sheet, WorldState, is_lockpick, lockpick_bonus, object_location, open_check};

    let Some(vfs) = vfs() else { return };
    let vfs = Arc::new(RwLock::new(vfs));
    let start = StartMap::load(&vfs.read().unwrap()).unwrap();
    let mut world = WorldState::default();
    let sheet = CharacterSheet { name: "Virgil".into(), ..Default::default() };
    world.add_player(Some(&start), PlayerId(1), CharacterBlueprint { sheet, gold: 0, items: vec![], prototype: None });
    let mut sim = SimApp::new(RulesData::default());
    sim.use_manual_clock();
    sim.app.insert_resource(SimVfs(vfs.clone()));
    sim.boot(world.0.unwrap());
    sim.update();
    sim.update();
    sim.take_deltas();
    // The nearest locked chest round the start.
    let map = sim.app.world().resource::<LoadedMap>();
    let skeleton_key = 15081 + arcanum_sim::BASIC_TO_PROTOTYPE;
    assert!(is_lockpick(skeleton_key, &map.prototypes), "the Auto Skeleton Key is a lockpick");
    let mut locked: Vec<(u64, GroundItemId, TileLoc, i32)> = Vec::new();
    for (&sector, objects) in &map.mobiles {
        for (index, object) in objects.iter().enumerate() {
            let Some(at) = object_location(object).filter(|_| object.object_type == TYPE_CONTAINER) else { continue };
            let flags = map.prototypes.int(object, "container_flags").unwrap_or(0);
            let id = GroundItemId::Container { sector, index: index as u32 };
            if open_check(flags, map.lock_day, false) == Err(arcanum_sim::OpenRefusal::Locked) {
                let d = (at.x.abs_diff(start.position.x) as u64).pow(2) + (at.y.abs_diff(start.position.y) as u64).pow(2);
                locked.push((d, id, at, map.lock_difficulty(LockTarget::Container(id))));
            }
        }
    }
    locked.sort_by_key(|c| c.0);
    let &(_, chest, at, difficulty) = locked.first().expect("a locked chest");
    eprintln!("{} locked chests; nearest at {at:?}, difficulty {difficulty}", locked.len());
    // A master of Pick Locks holding the Auto Skeleton Key stands at it.
    {
        let world = sim.app.world_mut();
        let mut q = world.query_filtered::<(&mut Position, &mut Sheet, &mut Inventory), bevy_ecs::query::With<PlayerControlled>>();
        let (mut position, mut sheet, mut inventory) = q.single_mut(world).unwrap();
        position.0 = at;
        sheet.0.base[1] = 20;
        sheet.0.skills[arcanum_sim::SKILL_PICK_LOCKS] = 5;
        inventory.items = vec![CarriedItem { prototype: skeleton_key, location: 0, quantity: 0, keys: vec![] }];
    }
    let map = sim.app.world().resource::<LoadedMap>();
    assert_eq!(lockpick_bonus(&Inventory { items: vec![CarriedItem { prototype: skeleton_key, location: 0, quantity: 0, keys: vec![] }], ..Default::default() }, Some(0), &map.prototypes), 10);
    let mut picked = false;
    for seq in 0..20 {
        sim.receive([(PlayerId(1), PlayerIntent::PickLock { target: LockTarget::Container(chest), tool: Some(0), seq })]);
        sim.update();
        if sim.take_deltas().iter().any(|(_, d)| *d == LedgerDelta::SkillResult { message: arcanum_sim::PICK_SUCCEEDED }) {
            picked = true;
            break;
        }
    }
    assert!(picked, "a 95 % pick gives within 20 tries");
    sim.receive([(PlayerId(1), PlayerIntent::PickUp { item: chest, seq: 0 })]);
    sim.update();
    assert!(sim.take_deltas().iter().any(|(_, d)| *d == LedgerDelta::PileOpened { pile: chest }), "and it opens");
}
