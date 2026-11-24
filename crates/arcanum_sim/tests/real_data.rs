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

/// A world for real scripts: object 1 is the only PC.
struct Scripted<'a> {
    vfs: &'a Vfs,
    library: arcanum_sim::ScriptLibrary,
    globals: arcanum_sim::ScriptGlobals,
    pc: arcanum_sim::PcScriptState,
    shown: Vec<String>,
}

impl arcanum_sim::ScriptContext for Scripted<'_> {
    type Object = u32;
    fn script(&mut self, number: u32) -> Option<std::sync::Arc<arcanum_formats::script::Script>> {
        self.library.get(self.vfs, number)
    }
    fn globals(&mut self) -> &mut arcanum_sim::ScriptGlobals {
        &mut self.globals
    }
    fn pc_state(&mut self, object: u32) -> Option<&mut arcanum_sim::PcScriptState> {
        (object == 1).then_some(&mut self.pc)
    }
    fn random(&mut self, low: i32, _high: i32) -> i32 {
        low
    }
    fn now(&self) -> arcanum_sim::GameTime {
        arcanum_sim::GameTime::at_hour(15)
    }
    fn dialog_text(&mut self, script: u32, line: i32, _reader: u32) -> Option<String> {
        self.library.dialog(self.vfs, script)?.text(line, false).map(str::to_string)
    }
    fn show_message(&mut self, _to: u32, text: String) {
        self.shown.push(text);
    }
}

#[test]
fn real_scripts_run_and_the_unbuilt_types_are_counted() {
    use arcanum_sim::{Attachment, HookResult, Invocation, Unsupported, run_hook};
    let Some(vfs) = vfs() else { return };
    let library = arcanum_sim::ScriptLibrary::new(&vfs);
    let numbers: Vec<u32> = library.index.files.keys().copied().collect();
    assert!(numbers.len() >= 2399, "{}", numbers.len());
    let mut world = Scripted { vfs: &vfs, library, globals: Default::default(), pc: Default::default(), shown: vec![] };
    let invocation = Invocation { attachee: 50, triggerer: 1, other: None, event: 1, start_line: 0 };
    // 01621Open_Bed_CM.scr: global flag 2273 decides.
    assert_eq!(run_hook(&mut world, &mut Attachment::new(0, 0, 1621), invocation).result, HookResult::SkipDefault);
    world.globals.set_flag(2273, true);
    assert_eq!(run_hook(&mut world, &mut Attachment::new(0, 0, 1621), invocation).result, HookResult::RunDefault);
    // Every script from its first line: what stops them.
    let (mut finished, mut stops) = (0, std::collections::BTreeMap::<String, usize>::new());
    for number in numbers {
        world.globals = Default::default();
        let run = run_hook(&mut world, &mut Attachment::new(0, 0, number), invocation);
        match run.unsupported.first() {
            None => finished += 1,
            Some(Unsupported::Condition { kind, .. }) => *stops.entry(format!("condition {kind:#x}")).or_default() += 1,
            Some(Unsupported::Action { kind, .. }) => *stops.entry(format!("action {kind:#x}")).or_default() += 1,
            Some(Unsupported::ObjectSet { kind, .. }) => *stops.entry(format!("set {kind:#x}")).or_default() += 1,
            Some(Unsupported::Trap { .. }) => *stops.entry("trap".into()).or_default() += 1,
        }
    }
    let mut top: Vec<_> = stops.into_iter().collect();
    top.sort_by_key(|t| std::cmp::Reverse(t.1));
    eprintln!("{finished} scripts ran to an end; stopped by: {:?}", &top[..top.len().min(15)]);
    assert!(finished > 0);
}


#[test]
fn doors_and_containers_near_the_start_carry_their_use_scripts() {
    use arcanum_formats::map::{MAP_LIST, parse_map_list, start_map};
    use arcanum_formats::mes::MesFile;
    use arcanum_formats::object::{TYPE_CONTAINER, TYPE_PORTAL, TYPE_SCENERY};
    use arcanum_sim::{EVENT_USE, Invocation, attachments, run_hook};
    let Some(vfs) = vfs() else { return };
    let list = parse_map_list(&MesFile::parse(&vfs.read(MAP_LIST).unwrap()).unwrap()).unwrap();
    let name = start_map(&list).unwrap().name.clone();
    let mut map = arcanum_sim::LoadedMap::default();
    map.open(Some(&vfs), &name);
    map.load_around(Some(&vfs), TileLoc { x: 90330, y: 84122 });
    let library = arcanum_sim::ScriptLibrary::new(&vfs);
    let mut world = Scripted { vfs: &vfs, library, globals: Default::default(), pc: Default::default(), shown: vec![] };
    let objects = map.sectors.values().flat_map(|s| s.objects.iter()).chain(map.mobiles.values().flatten());
    let mut found = std::collections::BTreeMap::<(u32, u32), usize>::new();
    let mut results = (0, 0, 0);
    for object in objects.filter(|o| [TYPE_PORTAL, TYPE_CONTAINER, TYPE_SCENERY].contains(&o.object_type)) {
        let Some(mut attachment) = attachments(object, &map.prototypes).get(&EVENT_USE).copied() else { continue };
        *found.entry((object.object_type, attachment.number)).or_default() += 1;
        let run = run_hook(&mut world, &mut attachment, Invocation { attachee: 50, triggerer: 1, other: None, event: EVENT_USE, start_line: 0 });
        match (run.result, run.unsupported.is_empty()) {
            (_, false) => results.2 += 1,
            (arcanum_sim::HookResult::SkipDefault, true) => {
                results.0 += 1;
                eprintln!("skips the default: script {} on type {}", attachment.number, object.object_type);
            }
            (arcanum_sim::HookResult::RunDefault, true) => results.1 += 1,
        }
    }
    eprintln!("use scripts (type, script) -> count: {found:?}; skip/run default/unsupported: {results:?}");
    assert!(!found.is_empty(), "the start map has use scripts (scenery 1274 among them)");
}


#[test]
fn the_inn_bed_script_tells_an_unpaying_pc_to_pay() {
    use arcanum_sim::{Attachment, HookResult, Invocation, run_hook};
    let Some(vfs) = vfs() else { return };
    let library = arcanum_sim::ScriptLibrary::new(&vfs);
    let mut world = Scripted { vfs: &vfs, library, globals: Default::default(), pc: Default::default(), shown: vec![] };
    let invocation = Invocation { attachee: 50, triggerer: 1, other: None, event: 1, start_line: 0 };
    // Counter 0 picks the inn: 1 reads global var 1043, the hour the room is paid until.
    let run = run_hook(&mut world, &mut Attachment::new(0, 1, 1990), invocation);
    assert_eq!(run.result, HookResult::RunDefault, "{run:?}");
    assert!(run.unsupported.is_empty(), "{run:?}");
    assert_eq!(world.shown, ["You must pay the Inn Keeper to use this bed."]);
    // Paid until a later hour (the game time here is day 0, 15:00): it sleeps.
    world.globals.set_var(1043, 20);
    assert_eq!(run_hook(&mut world, &mut Attachment::new(0, 1, 1990), invocation).result, HookResult::SkipDefault);
}

#[test]
fn virgil_greets_the_pc_at_the_crash_site() {
    use arcanum_rules::{CharacterBlueprint, CharacterSheet, RulesData};
    use arcanum_sim::{EVENT_DIALOGUE, LedgerDelta, LoadedMap, NetId, NpcSource, PlayerId, PlayerIntent, PlayerSave, SimApp, SimVfs, WorldSave};
    use std::sync::{Arc, RwLock};
    let Some(vfs) = vfs() else { return };
    let start = StartMap::load(&vfs).unwrap();
    let rules = RulesData::load(&vfs).unwrap();
    // Find Virgil: the NPC whose dialogue script is 1324 ("Virgil").
    let mut map = LoadedMap::default();
    map.open(Some(&vfs), &start.map);
    map.load_around(Some(&vfs), start.position);
    let virgil = map.mobiles.iter().flat_map(|(&s, objects)| objects.iter().enumerate().map(move |(i, o)| (s, i, o))).find(|(_, _, o)| arcanum_sim::attachments(o, &map.prototypes).get(&EVENT_DIALOGUE).is_some_and(|a| a.number == 1324));
    let Some((sector, index, object)) = virgil else { panic!("no Virgil near the start") };
    let at = arcanum_sim::object_location(object).unwrap();
    // A PC beside him.
    let mut save = WorldSave::new_game(&start);
    let sheet = CharacterSheet { name: "Virgil's Hero".into(), ..Default::default() };
    let here = TileLoc { x: at.x + 1, y: at.y };
    save.players.insert(PlayerId(1), PlayerSave { blueprint: CharacterBlueprint { sheet, gold: 0, items: vec![], prototype: None }, position: here, inventory: None, script_state: Default::default() });
    let mut sim = SimApp::new(rules);
    sim.use_manual_clock();
    sim.app.insert_resource(SimVfs(Arc::new(RwLock::new(vfs))));
    sim.boot(save);
    for _ in 0..3 {
        sim.update();
    }
    sim.take_deltas();
    let npc = {
        let world = sim.app.world_mut();
        world.query::<(&NetId, &NpcSource)>().iter(world).find(|(_, s)| s.sector == sector && s.index == index as u32).map(|(n, _)| *n)
    };
    let npc = npc.expect("Virgil spawned");
    sim.receive([(PlayerId(1), PlayerIntent::Talk { npc, seq: 0 })]);
    sim.update();
    let deltas = sim.take_deltas();
    let line = deltas.iter().find_map(|(_, d)| match d {
        LedgerDelta::FloatText { speaker, text, voice, .. } if *speaker == npc => Some((text.clone(), voice.clone())),
        _ => None,
    });
    let options = deltas.iter().find_map(|(_, d)| match d {
        LedgerDelta::DialogueOptions { options, .. } => Some(options.clone()),
        _ => None,
    });
    let reports = sim.app.world().resource::<arcanum_sim::ScriptReports>().0.clone();
    let dialogue_reports = sim.app.world().resource::<arcanum_sim::DialogueReports>().0.clone();
    eprintln!("Virgil says {line:?}; options {options:?}; script reports {reports:?}; dialogue reports {dialogue_reports:?}");
    let (text, voice) = line.expect("Virgil speaks");
    assert!(text.starts_with("I can't believe it"), "{text}");
    assert_eq!(voice.as_deref(), Some("sound/speech/01324/v1_m.mp3"));
    // His reaction gates the options; one of each pair shows.
    let options = options.expect("options");
    assert_eq!(options, ["What are you going on about?", "By the gods, man! I almost died here! Didn't you see the crash?"]);
    sim.receive([(PlayerId(1), PlayerIntent::Choose { option: 0 })]);
    sim.update();
    let deltas = sim.take_deltas();
    let next = deltas.iter().find_map(|(_, d)| match d {
        LedgerDelta::FloatText { speaker, text, .. } if *speaker == npc => Some(text.clone()),
        _ => None,
    });
    assert!(next.as_deref().is_some_and(|t| t.starts_with("[The man's jaw drops open.]")), "{next:?}");
    assert!(deltas.iter().any(|(_, d)| matches!(d, LedgerDelta::DialogueOptions { options, .. } if !options.is_empty())), "{deltas:?}");
}

/// Every greeting (class and race blocks, both genders, dumb or not, met or not, each reaction
/// level) finds a line in the real `gd_*` files, but for one gap in the data.
#[test]
fn every_generic_greeting_has_a_line() {
    use arcanum_rules::{CharacterSheet, FEMALE, MALE, RulesData};
    use arcanum_sim::{Attachment, GenericLines, HostRng, Inventory, LedgerDelta, NetId, NpcInfo, NpcReaction, PcScriptState, ReputationEffects, Say, ScriptGlobals, Talk};

    let Some(vfs) = vfs() else { return };
    let rules = RulesData::default();
    let reputations = ReputationEffects::default();
    let mut generic = GenericLines::default();
    let mut rng = HostRng::default();
    let (mut tried, mut missing) = (0, Vec::new());
    for npc_male in [true, false] {
        for dumb in [false, true] {
            for class in 0..11 {
                for pc_gender in [MALE, FEMALE] {
                    for pc_race in 0..8u8 {
                        for (level, base) in [101, 90, 70, 50, 30, 10, 0].into_iter().enumerate() {
                            for met in [false, true] {
                                let sheet = CharacterSheet { gender: pc_gender, race: pc_race, ..Default::default() };
                                let (mut inventory, mut pc, mut globals, mut attachment): (Inventory, PcScriptState, ScriptGlobals, Attachment) = Default::default();
                                // Looks and race do not count, so the base is the reaction.
                                let reaction = NpcReaction { base, race: 0, origin: 0, faction: 0, only_reputation: true };
                                let mut talk = Talk { rules: &rules, sheet: &sheet, inventory: &mut inventory, pc: &mut pc, globals: &mut globals, npc: &mut attachment, met, npc_key: "npc".into(), npc_reaction: reaction, reputations: &reputations, gold_changed: false, unbuilt: Vec::new() };
                                assert_eq!(arcanum_sim::level(talk.reaction()), level as u32);
                                let info = NpcInfo { name: "N".into(), male: npc_male, reaction, class, dumb, overrides: None };
                                let mut say = Say { npc_net: NetId(1), dialog: 0, pc_name: "P", npc: &info, generic: &mut generic, vfs: Some(&vfs), rng: &mut rng, deltas: Vec::new(), jumps: 0 };
                                say.greeting("", &mut talk);
                                tried += 1;
                                if !say.deltas.iter().any(|d| matches!(d, LedgerDelta::FloatText { text, .. } if !text.is_empty())) {
                                    missing.push((npc_male, dumb, class, pc_gender, pc_race, level, met));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!("greetings: {tried} tried, {} without a line", missing.len());
    // gd_cls_m2m's wizard block (8100-8149) has no 8113: the original counts 45 lines and picks
    // 8100 + r, so one pick in 45 finds nothing there, as here.
    missing.retain(|&(npc_male, dumb, class, pc_gender, _, level, met)| !(npc_male && !dumb && class == 2 && pc_gender == MALE && level == 0 && !met));
    assert!(missing.is_empty(), "{:?}", &missing[..missing.len().min(20)]);
}
