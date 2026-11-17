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
    Some(vfs)
}

#[test]
fn new_games_start_where_the_original_does() {
    let Some(vfs) = vfs() else { return };
    let start = StartMap::load(&vfs).unwrap();
    assert_eq!(start, StartMap { map: "Arcanum1-024-fixed".into(), position: TileLoc { x: 92958, y: 82592 } });
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
    let world = sim.app.world_mut();
    let positions: Vec<TileLoc> = world.query::<&Position>().iter(world).map(|p| p.0).collect();
    assert_eq!(positions, vec![start.position]);
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
    eprintln!("{found} tiles reached, {blocked} mask-blocked, of 41 x 41 around the start");
    assert!(found > 100, "{found}");
}
