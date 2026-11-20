//! Critters on the map: their sprites, stand and walk animations, and walking between tiles.
//!
//! The host announces each PC with a `Spawn` (its tile, art id and speed stat) and moves it with
//! `Move` (other players' PCs) and `MoveCorrection`. The local PC walks on the client at once
//! ([`WalkLocalPc`], set by a left click on the map): the client sends the path to the host and
//! walks it without waiting.
//!
//! A correction puts the actor on the host's tile, and its sprite slides there from where it was
//! drawn, at its walking speed, instead of jumping. For the local PC, a correction answering an
//! older move than the last one sent is ignored (the answer to the newer move decides), and
//! after a correction the client plans once more to the same goal from the host's tile.
//!
//! A walker steps from tile centre to tile centre, each step taking as long as its
//! [`Gait`] says, which is also the host's timing. Its sprite plays the walk art in the step's
//! direction at the gait's frame time, and the stand art (same direction) once it stops. The
//! sprite sits at the tile centre on its frame's hotspot; it is drawn after the objects on its
//! tile, at the depth of whichever of the two tiles of its step is lower on screen.

use std::collections::{HashMap, VecDeque};

use arcanum_formats::art_id::{ANIM_STAND, ArtId, CritterArt};
use arcanum_formats::map::TileLoc;
use arcanum_sim::{Gait, LedgerDelta, NetId, PlayerIntent, find_path, rotation_of_step, walk_art, with_anim};
use bevy::prelude::*;

use crate::animation::ArtAnimation;
use crate::art_loader::ArtFrames;
use crate::isometric::{tile_to_world, world_to_tile};
use crate::main_menu::AppState;
use crate::sim_bridge::{HostLink, LocalPc};
use crate::vfs_asset::VfsResource;
use crate::world_render::{ClientMap, DEPTH_TILE_ORDER, object_depth};

pub struct ActorsPlugin;

impl Plugin for ActorsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActorDeltas>().init_resource::<WalkLocalPc>().add_systems(OnExit(AppState::InGame), forget_actors).add_systems(
            Update,
            (click_to_move, apply_actor_deltas, start_local_walk, walk_actors, place_actors).chain().run_if(in_state(AppState::InGame)),
        );
    }
}

/// Deltas about actors, kept from the bridge until the world is up to apply them.
#[derive(Resource, Debug, Default)]
pub struct ActorDeltas(pub Vec<LedgerDelta>);

impl ActorDeltas {
    /// Whether this module handles `delta`.
    pub fn wants(delta: &LedgerDelta) -> bool {
        matches!(delta, LedgerDelta::Spawn { .. } | LedgerDelta::Move { .. } | LedgerDelta::MoveCorrection { .. } | LedgerDelta::Despawn { .. })
    }
}

/// A tile for the local PC to walk to: the client plans the path, walks it and tells the host.
#[derive(Resource, Debug, Default)]
pub struct WalkLocalPc {
    pub goal: Option<TileLoc>,
    /// Numbering of the moves sent.
    seq: u32,
    /// The goal of the last move sent, and whether it was already planned again after a
    /// correction.
    last_goal: Option<(TileLoc, bool)>,
}

impl WalkLocalPc {
    /// Takes the host's correction of the local PC to `position`, answering move `seq`. Returns
    /// false if it answers an older move than the last one sent, so is to be ignored. Otherwise,
    /// the first correction after a move plans again to its goal, from `position`.
    pub fn corrected(&mut self, seq: u32, position: TileLoc) -> bool {
        if seq < self.seq {
            return false;
        }
        if let Some((goal, false)) = self.last_goal
            && goal != position
        {
            self.goal = Some(goal);
            self.last_goal = Some((goal, true));
        }
        true
    }

    /// Records that move `seq + 1` to `goal` is being sent and returns its number.
    fn sending(&mut self, goal: TileLoc) -> u32 {
        let again = matches!(self.last_goal, Some((g, true)) if g == goal);
        self.seq += 1;
        self.last_goal = Some((goal, again));
        self.seq
    }

    /// A new goal from the player.
    fn click(&mut self, goal: TileLoc) {
        self.goal = Some(goal);
        self.last_goal = None;
    }
}

/// A critter the host told us about.
#[derive(Component, Debug, Clone)]
pub struct Actor {
    pub net_id: NetId,
    /// Standing, rotation 0; the rotation in use is [`Actor::rotation`].
    pub art: ArtId,
    pub gait: Gait,
    pub at: TileLoc,
    pub rotation: usize,
    pub steps: VecDeque<TileLoc>,
    /// Seconds into the step to `steps[0]`.
    pub progress: f64,
    /// Where the sprite is drawn relative to its place on the path, in world units: left by a
    /// correction, and shrinking to nothing at walking speed.
    pub slide: Vec2,
}

impl Actor {
    pub fn walking(&self) -> bool {
        !self.steps.is_empty()
    }

    /// How far through the current step, 0 to 1.
    pub fn step_fraction(&self) -> f32 {
        let Some(&next) = self.steps.front() else { return 0.0 };
        let total = self.gait.step_seconds(self.at, next);
        if total <= 0.0 { 0.0 } else { (self.progress / total).clamp(0.0, 1.0) as f32 }
    }

    /// Walks on by `dt` seconds. Returns true if it changed tile.
    pub fn advance(&mut self, dt: f64) -> bool {
        if self.steps.is_empty() {
            self.progress = 0.0;
            return false;
        }
        self.progress += dt;
        let mut moved = false;
        while let Some(&next) = self.steps.front() {
            let needed = self.gait.step_seconds(self.at, next);
            if self.progress < needed {
                break;
            }
            self.progress -= needed;
            self.at = next;
            self.steps.pop_front();
            moved = true;
        }
        if let Some(rotation) = self.steps.front().and_then(|&next| rotation_of_step(self.at, next)) {
            self.rotation = rotation;
        }
        if self.steps.is_empty() {
            self.progress = 0.0;
        }
        moved
    }

    /// Starts along `path` (the tiles after the current one).
    pub fn walk(&mut self, path: impl IntoIterator<Item = TileLoc>) {
        self.steps = path.into_iter().collect();
        self.progress = 0.0;
        if let Some(rotation) = self.steps.front().and_then(|&next| rotation_of_step(self.at, next)) {
            self.rotation = rotation;
        }
    }

    /// Where the actor is drawn relative to the centre of its tile, in world units: between its
    /// tile and the next.
    pub fn drawn_offset(&self) -> Vec2 {
        let base = match self.steps.front() {
            Some(&next) => tile_to_world(next, self.at) * self.step_fraction(),
            None => Vec2::ZERO,
        };
        base + self.slide
    }

    /// Puts the actor on `position`, standing, its sprite sliding there from where it is drawn.
    pub fn correct(&mut self, position: TileLoc) {
        self.slide = self.drawn_offset() + tile_to_world(self.at, position);
        self.at = position;
        self.walk([]);
    }

    /// Walking speed in world units a second (the mean over the rotations).
    fn speed(&self) -> f32 {
        let per_frame = self.gait.pixels_per_frame.iter().sum::<f64>() / 8.0;
        (per_frame * 1000.0 / f64::from(self.gait.frame_millis.max(1))) as f32
    }

    /// Shrinks [`Actor::slide`] by `dt` seconds of walking.
    fn settle(&mut self, dt: f32) {
        let left = self.slide.length() - self.speed() * dt;
        self.slide = if left <= 0.0 { Vec2::ZERO } else { self.slide.normalize() * left };
    }

    /// The art to show now: walking or standing, in the current rotation.
    pub fn current_art(&self) -> ArtId {
        let anim = if self.walking() { walk_art(self.art) } else { with_anim(self.art, ANIM_STAND) };
        CritterArt { rotation: self.rotation as u32, ..CritterArt::from_id(anim) }.id()
    }
}

fn forget_actors(mut deltas: ResMut<ActorDeltas>, mut walk: ResMut<WalkLocalPc>) {
    deltas.0.clear();
    walk.goal = None;
    walk.last_goal = None;
}

#[allow(clippy::too_many_arguments)]
fn apply_actor_deltas(
    mut commands: Commands,
    mut deltas: ResMut<ActorDeltas>,
    assets: Res<AssetServer>,
    vfs: Option<Res<VfsResource>>,
    pc: Res<LocalPc>,
    mut walk: ResMut<WalkLocalPc>,
    mut actors: Query<(Entity, &mut Actor)>,
) {
    if deltas.0.is_empty() {
        return;
    }
    let local = pc.0.map(|(id, _)| id);
    let mut by_id: HashMap<NetId, Entity> = actors.iter().map(|(e, a)| (a.net_id, e)).collect();
    for delta in deltas.0.drain(..) {
        match delta {
            LedgerDelta::Spawn { net_id, position, art, speed, .. } => {
                if by_id.contains_key(&net_id) {
                    continue;
                }
                let art = ArtId(art);
                let gait = match vfs.as_ref().and_then(|v| v.0.read().ok()) {
                    Some(vfs) => Gait::load(art, speed, |p| vfs.read(p).ok()),
                    None => Gait::load(art, speed, |_| None),
                };
                let rotation = CritterArt::from_id(art).rotation as usize;
                let actor = Actor { net_id, art, gait, at: position, rotation, steps: VecDeque::new(), progress: 0.0, slide: Vec2::ZERO };
                let path = CritterArt::from_id(actor.current_art()).path();
                let handle: Handle<ArtFrames> = assets.load(path);
                let entity = commands
                    .spawn((
                        ArtAnimation::new(handle, rotation),
                        actor,
                        Sprite::default(),
                        bevy::sprite::Anchor::CENTER,
                        Transform::default(),
                        DespawnOnExit(AppState::InGame),
                    ))
                    .id();
                by_id.insert(net_id, entity);
            }
            LedgerDelta::Move { net_id, path } if Some(net_id) != local => {
                if let Some(Ok((_, mut a))) = by_id.get(&net_id).map(|&e| actors.get_mut(e)) {
                    // The path may start on the actor's tile or the next one along.
                    let skip = usize::from(path.first() == Some(&a.at));
                    a.walk(path.into_iter().skip(skip));
                }
            }
            LedgerDelta::MoveCorrection { net_id, position, seq } => {
                if Some(net_id) == local {
                    if !walk.corrected(seq, position) {
                        continue;
                    }
                    info!("the host stopped the PC at {position:?} (move {seq})");
                }
                if let Some(Ok((_, mut a))) = by_id.get(&net_id).map(|&e| actors.get_mut(e)) {
                    a.correct(position);
                }
            }
            LedgerDelta::Despawn { net_id } => {
                if let Some(e) = by_id.remove(&net_id) {
                    commands.entity(e).despawn();
                }
            }
            _ => {}
        }
    }
}

/// A click to make once the world is up, at this window position (logical pixels): for
/// screenshot checks, which take no input. It waits `frames` frames after the sectors are in,
/// for the camera to reach the PC.
#[derive(Resource, Debug, Clone, Copy)]
pub struct DebugClick {
    pub at: Vec2,
    pub frames: u32,
}

/// A left click on the map walks the local PC to the tile under the cursor (0x54de50, the game
/// window's mouse handler). With Shift held the click does nothing, as in the original.
#[allow(clippy::too_many_arguments)]
fn click_to_move(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    debug: Option<ResMut<DebugClick>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    client: Option<Res<ClientMap>>,
    mut walk: ResMut<WalkLocalPc>,
) {
    let Some(origin) = client.filter(|c| !c.map.sectors.is_empty()).and_then(|c| c.origin) else { return };
    let cursor = match debug {
        Some(mut click) if click.frames > 0 => {
            click.frames -= 1;
            return;
        }
        Some(click) => {
            commands.remove_resource::<DebugClick>();
            click.at
        }
        None if buttons.just_pressed(MouseButton::Left) && !keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) => {
            let Some(cursor) = windows.iter().find_map(Window::cursor_position) else { return };
            cursor
        }
        None => return,
    };
    let Some(world) = cameras.iter().find_map(|(camera, at)| camera.viewport_to_world_2d(at, cursor).ok()) else { return };
    if let Some(goal) = world_to_tile(world, origin) {
        debug!("click at {cursor} goes to {goal:?}");
        walk.click(goal);
    }
}

/// Plans the local PC's path to [`WalkLocalPc::goal`] on the client's map, walks it and sends it
/// to the host.
fn start_local_walk(mut walk: ResMut<WalkLocalPc>, client: Option<Res<ClientMap>>, pc: Res<LocalPc>, link: Option<Res<HostLink>>, mut actors: Query<&mut Actor>) {
    let Some(goal) = walk.goal else { return };
    let (Some(client), Some((net_id, _))) = (client, pc.0) else { return };
    if client.map.sectors.is_empty() {
        return;
    }
    let Some(mut actor) = actors.iter_mut().find(|a| a.net_id == net_id) else { return };
    walk.goal = None;
    // Mid-step, the path starts from the tile being stepped onto.
    let from = actor.steps.front().copied().unwrap_or(actor.at);
    let path = match find_path(&client.map, from, goal) {
        Ok(path) => path,
        Err(e) => {
            warn!("no path to {goal:?}: {e:?}");
            return;
        }
    };
    let seq = walk.sending(goal);
    let mut sent = vec![from];
    sent.extend(&path);
    if let Some(link) = link
        && let Err(e) = link.0.send_intent(PlayerIntent::Move { seq, path: sent })
    {
        warn!("cannot reach the host: {e}");
    }
    if actor.steps.front() == Some(&from) {
        // Finish the step under way, then follow the new path.
        actor.steps.truncate(1);
        actor.steps.extend(path);
    } else {
        actor.walk(path);
    }
}

/// Walks every actor on and keeps [`LocalPc`] on the local PC's tile.
fn walk_actors(time: Res<Time>, mut pc: ResMut<LocalPc>, mut actors: Query<&mut Actor>) {
    let dt = f64::from(time.delta_secs());
    for mut actor in &mut actors {
        if actor.slide != Vec2::ZERO {
            actor.settle(time.delta_secs());
        }
        if actor.advance(dt)
            && let Some((id, at)) = pc.0
            && id == actor.net_id
            && at != actor.at
        {
            pc.0 = Some((id, actor.at));
        }
    }
}

/// Puts each actor's sprite between its tiles and gives it the right art.
fn place_actors(client: Option<Res<ClientMap>>, assets: Res<AssetServer>, mut actors: Query<(&Actor, &mut ArtAnimation, &mut Transform)>) {
    let Some(origin) = client.and_then(|c| c.origin) else { return };
    for (actor, mut animation, mut transform) in &mut actors {
        let here = tile_to_world(actor.at, origin);
        let pos = here + actor.drawn_offset();
        let depth_tile = match actor.steps.front() {
            Some(&next) if tile_to_world(next, origin).y < here.y => next,
            _ => actor.at,
        };
        transform.translation = pos.extend(object_depth(depth_tile, origin, false, DEPTH_TILE_ORDER - 1));
        let path = CritterArt::from_id(actor.current_art()).path();
        let walking = actor.walking();
        let handle = assets.get_handle::<ArtFrames>(&path).unwrap_or_else(|| assets.load(path));
        animation.set(&handle, actor.rotation);
        animation.frame_millis = walking.then_some(actor.gait.frame_millis);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(x: u32, y: u32) -> TileLoc {
        TileLoc { x, y }
    }

    fn actor() -> Actor {
        let art = arcanum_sim::pc_art(0, true);
        Actor { net_id: NetId(0), art, gait: Gait::human(17), at: t(10, 10), rotation: 4, steps: VecDeque::new(), progress: 0.0, slide: Vec2::ZERO }
    }

    #[test]
    fn an_actor_walks_at_its_gait_and_faces_its_steps() {
        let mut a = actor();
        a.walk([t(9, 10), t(8, 11)]);
        assert_eq!(a.rotation, 1, "up-right");
        assert_eq!(CritterArt::from_id(a.current_art()).path(), "art/critter/hmm/hmmuwxab.art");
        let first = a.gait.step_seconds(t(10, 10), t(9, 10));
        assert!(!a.advance(first * 0.5));
        assert!((a.step_fraction() - 0.5).abs() < 1e-4);
        assert!(a.advance(first * 0.5 + 1e-9));
        assert_eq!((a.at, a.rotation), (t(9, 10), 2), "next step goes right");
        a.advance(10.0);
        assert_eq!(a.at, t(8, 11));
        assert!(!a.walking());
        assert_eq!(a.rotation, 2, "keeps facing the last step");
        assert_eq!(CritterArt::from_id(a.current_art()).path(), "art/critter/hmm/hmmuwxaa.art", "standing");
    }

    #[test]
    fn a_correction_stops_the_actor_where_the_host_says() {
        let mut a = actor();
        a.walk([t(11, 10), t(12, 10)]);
        let step = a.gait.step_seconds(t(10, 10), t(11, 10));
        a.advance(step / 2.0);
        let drawn = tile_to_world(t(10, 10), t(0, 0)) + a.drawn_offset();
        a.correct(t(10, 11));
        assert!(!a.walking());
        assert_eq!(a.step_fraction(), 0.0);
        assert_eq!(tile_to_world(t(10, 11), t(0, 0)) + a.drawn_offset(), drawn, "drawn where it was");
        let distance = a.slide.length();
        a.settle(0.1);
        assert!(a.slide.length() < distance, "slides towards the host's tile");
        a.settle(10.0);
        assert_eq!(a.slide, Vec2::ZERO, "and stops there");
    }

    #[test]
    fn only_the_newest_move_is_corrected_and_planned_once_more() {
        let mut w = WalkLocalPc::default();
        w.click(t(20, 20));
        w.goal = None;
        assert_eq!(w.sending(t(20, 20)), 1);
        assert_eq!(w.sending(t(20, 20)), 2, "a second click before the answer");
        assert!(!w.corrected(1, t(11, 10)), "answers the older move");
        assert_eq!(w.goal, None);
        assert!(w.corrected(2, t(11, 10)));
        assert_eq!(w.goal, Some(t(20, 20)), "plans again from the host's tile");
        w.goal = None;
        assert_eq!(w.sending(t(20, 20)), 3);
        assert!(w.corrected(3, t(12, 10)));
        assert_eq!(w.goal, None, "only once");
        w.click(t(30, 30));
        w.goal = None;
        w.sending(t(30, 30));
        assert!(w.corrected(4, t(30, 30)));
        assert_eq!(w.goal, None, "already at the goal");
    }
}
