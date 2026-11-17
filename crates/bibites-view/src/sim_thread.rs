//! The simulation on a thread of its own (`--sim-thread`).
//!
//! A world is ticked by one thread at a time, and a tick of a big world takes 12 ms or more, so a window that needs the world
//! in every frame (to look for what is under the cursor, to write the HUD, to draw it) waits for the tick in progress in every
//! frame and cannot draw faster than the simulation runs. This module lets the window *not* need the world while it only
//! watches: the world moves to a worker thread, which ticks it at the speed the window asks for, and publishes what the window
//! draws instead (the display list built for a camera a little wider than the window's, the HUD statistics, the simulated
//! time). The window draws the latest of those with its own camera at its own frame rate. Whenever the window needs the world
//! (a click, a key, a panel, a pause) it asks for it back, which takes at most one tick, and carries on as before.
//!
//! The worker's ticks are the world's own, one after the other, so the results are those of a world ticked in the window's loop.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bibites_core::ids::{BibiteId, PelletId};
use bibites_core::math::Vec2;
use bibites_core::world::{ScenarioData, World};

use crate::app::HudInfo;
use crate::assets::GameAssets;
use crate::render::{Camera, Scene, SceneOptions, ToolView, build_scene};
use crate::tooltip::Body;

/// How often the worker publishes a snapshot at the most.
const PUBLISH_PERIOD: Duration = Duration::from_micros(16_000);
/// A snapshot costs the worker time it does not spend ticking. A world that leaves the worker no time to sleep still gets a
/// snapshot after this many times what the last took (a few milliseconds for a big world), so that publishing takes about a
/// seventh of the worker's time at the most; a world that is ahead of the clock pays for its snapshots with the time the
/// worker sleeps.
const PUBLISH_COST_SHARE: u32 = 6;
/// The most sleeping time the worker keeps to pay for snapshots with.
const SPARE_CAP: Duration = Duration::from_millis(100);
/// The longest the worker keeps ticking to catch up before it looks at the clock, the HUD and the snapshots again.
const BURST: Duration = Duration::from_millis(40);
/// An idle worker (ahead of the clock) sleeps until its next tick or snapshot is due, for at least this long...
const IDLE_SLEEP_MIN: Duration = Duration::from_micros(100);
/// ...and at most this long (so that it looks at the stop flag and the speed often).
const IDLE_SLEEP_MAX: Duration = Duration::from_millis(2);
/// How often the HUD statistics are collected (a scan over the population).
const HUD_PERIOD: Duration = Duration::from_millis(200);
/// How much wider than the window the camera of the published display list is (each side), so that the window can pan
/// and zoom out between two snapshots without showing an edge.
const CAMERA_MARGIN: f32 = 1.25;

/// What the window asks the worker to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneRequest {
    /// The window's camera.
    pub camera: Camera,
    /// The selected bibite, if any.
    pub selected: Option<BibiteId>,
    /// Which overlays to draw.
    pub options: SceneOptions,
    /// The tools' placement state.
    pub tool_view: ToolView,
    /// The world point under the pointer: the thread says what the pellet tooltip needs to know about it.
    pub hover: Option<Vec2>,
}

/// What the world says about the point under the pointer (`App::update_tooltip` asks it every frame).
#[derive(Clone, Debug)]
pub struct Hover {
    /// The point that was asked about, in world units.
    pub point: Vec2,
    /// The pellet the pointer rests on, if no body covers it, with the text of its tooltip.
    pub pellet: Option<(PelletId, Body)>,
}

/// The pellet tooltip's question about `point`, answered from `world`.
#[must_use]
pub fn hover_answer(world: &World, point: Vec2) -> Hover {
    let pellet = crate::tooltip::pellet_tooltip_target(world, point)
        .and_then(|id| crate::tooltip::pellet_body(world, id).map(|body| (id, body)));
    Hover { point, pellet }
}

/// What the worker publishes for the window.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Increases with every snapshot.
    pub seq: u64,
    /// The display list, in world coordinates, built for a camera [`CAMERA_MARGIN`] times the window's.
    pub scene: Scene,
    /// Simulated seconds.
    pub sim_time: f64,
    /// The world's tick.
    pub tick: u64,
    /// The population statistics of the HUD.
    pub hud: HudInfo,
    /// The answer to the request's `hover` point.
    pub hover: Option<Hover>,
    /// Live pellets (for the frame log).
    pub pellets: usize,
    /// Live bibites (for the frame log).
    pub bibites: usize,
}

/// State shared between the window and the worker.
struct Shared {
    /// The window wants the world back.
    stop: AtomicBool,
    /// The time scale to run at (`f32` bits).
    engine: AtomicU32,
    /// Ticks the worker has run since it started.
    ticks: AtomicU64,
    /// What to draw.
    request: Mutex<Option<SceneRequest>>,
    /// The newest snapshot.
    latest: Mutex<Option<Arc<Snapshot>>>,
}

enum Command {
    Run(Box<World>),
    Shutdown,
    /// Tests: the worker panics, as a tick that hit a bug would.
    #[cfg(test)]
    Panic,
}

enum Event {
    Ready,
    Returned(Box<World>),
}

/// The worker thread and the channels to it.
pub struct SimThread {
    commands: Sender<Command>,
    events: Receiver<Event>,
    shared: Arc<Shared>,
    handle: Option<std::thread::JoinHandle<()>>,
    ready: bool,
    running: bool,
}

/// An empty world with the settings of `world`: what stands in the window's `World` while the real one is on the worker. Code
/// that reads settings (the camera's limits, the labels of the HUD) finds them; code that looks for bibites finds none.
#[must_use]
pub fn husk_like(world: &World) -> World {
    let mut small = world.settings.clone();
    // A small square: the spatial hash of a husk is as big as its world.
    small.independents.simulation_size = 100.0;
    let mut husk = World::new(small, ScenarioData::default(), 0).expect("an empty world builds");
    husk.settings = world.settings.clone();
    // A world loaded while the real one is away takes its worker count from the husk.
    husk.set_threads(world.threads());
    husk
}

/// The camera the display list is built for: the window's, [`CAMERA_MARGIN`] times as wide and as high.
pub(crate) fn widened(camera: Camera) -> Camera {
    let mut wide = camera;
    wide.set_screen(camera.screen * CAMERA_MARGIN);
    wide
}

impl SimThread {
    /// Starts the worker. It loads its own copy of the game assets from `assets_root` (the display lists need the sprite
    /// sizes), which takes a moment: [`SimThread::is_ready`] says when it is done.
    #[must_use]
    pub fn spawn(assets_root: Option<PathBuf>) -> Self {
        let (commands, command_rx) = channel();
        let (event_tx, events) = channel();
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            engine: AtomicU32::new(1.0_f32.to_bits()),
            ticks: AtomicU64::new(0),
            request: Mutex::new(None),
            latest: Mutex::new(None),
        });
        let worker_shared = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("simulation".to_owned())
            // A tick of a big world runs on this thread when the pool has one thread.
            .stack_size(16 << 20)
            .spawn(move || worker(command_rx, event_tx, worker_shared, assets_root))
            .expect("the simulation thread starts");
        Self {
            commands,
            events,
            shared,
            handle: Some(handle),
            ready: false,
            running: false,
        }
    }

    /// Whether the worker has loaded what it needs and can take a world.
    pub fn is_ready(&mut self) -> bool {
        if !self.ready
            && let Ok(Event::Ready) = self.events.try_recv()
        {
            self.ready = true;
        }
        self.ready
    }

    /// Whether the world is on the worker.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Whether the worker thread ended while it should be alive: a tick panicked (the world went down with it).
    #[must_use]
    pub fn has_died(&self) -> bool {
        self.handle
            .as_ref()
            .is_some_and(|handle| handle.is_finished())
    }

    /// Tests: makes the worker panic.
    #[cfg(test)]
    fn make_it_panic(&self) {
        let _ = self.commands.send(Command::Panic);
    }

    /// Hands the world to the worker, which ticks it until [`SimThread::reclaim`].
    ///
    /// # Panics
    ///
    /// Panics when the worker is not ready or already has a world.
    pub fn run(&mut self, world: Box<World>) {
        assert!(self.ready && !self.running, "the worker is not free");
        self.shared.stop.store(false, Ordering::Release);
        self.running = true;
        self.commands
            .send(Command::Run(world))
            .expect("the simulation thread is alive");
    }

    /// Takes the world back: the worker finishes the tick it is in and returns it. Waits for that.
    ///
    /// # Panics
    ///
    /// Panics when the worker has no world, or died (a tick panicked).
    pub fn reclaim(&mut self) -> Box<World> {
        assert!(self.running, "the worker has no world");
        self.shared.stop.store(true, Ordering::Release);
        loop {
            match self.events.recv() {
                Ok(Event::Returned(world)) => {
                    self.running = false;
                    return world;
                }
                Ok(Event::Ready) => self.ready = true,
                Err(_) => panic!("the simulation thread died"),
            }
        }
    }

    /// The time scale the worker runs at.
    pub fn set_engine(&self, speed: f32) {
        self.shared
            .engine
            .store(speed.max(0.0).to_bits(), Ordering::Relaxed);
    }

    /// What the worker draws from now on.
    pub fn set_request(&self, request: SceneRequest) {
        *lock(&self.shared.request) = Some(request);
    }

    /// Tests: the worker publishes nothing until it is asked to draw again.
    #[cfg(test)]
    fn clear_request(&self) {
        *lock(&self.shared.request) = None;
    }

    /// Tests: what the worker is asked to draw.
    #[cfg(test)]
    pub(crate) fn request(&self) -> Option<SceneRequest> {
        *lock(&self.shared.request)
    }

    /// The newest snapshot, if the worker has published one.
    #[must_use]
    pub fn latest(&self) -> Option<Arc<Snapshot>> {
        lock(&self.shared.latest).clone()
    }

    /// Forgets the snapshots (a world that was reclaimed has moved on).
    pub fn clear_snapshot(&self) {
        *lock(&self.shared.latest) = None;
    }

    /// Ticks the worker has run since it started.
    #[must_use]
    pub fn ticks(&self) -> u64 {
        self.shared.ticks.load(Ordering::Relaxed)
    }
}

impl Drop for SimThread {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let _ = self.commands.send(Command::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn worker(
    commands: Receiver<Command>,
    events: Sender<Event>,
    shared: Arc<Shared>,
    assets_root: Option<PathBuf>,
) {
    let assets = assets_root.and_then(|root| GameAssets::load(&root).ok());
    let _ = events.send(Event::Ready);
    while let Ok(command) = commands.recv() {
        match command {
            Command::Shutdown => break,
            #[cfg(test)]
            Command::Panic => panic!("the simulation thread was told to panic"),
            Command::Run(mut world) => {
                run(&mut world, &shared, assets.as_ref());
                if events.send(Event::Returned(world)).is_err() {
                    break;
                }
            }
        }
    }
}

/// Ticks `world` at the requested speed until the window asks for it back, publishing snapshots as it goes.
fn run(world: &mut World, shared: &Shared, assets: Option<&GameAssets>) {
    let fixed = f64::from(world.settings.fixed_dt());
    let mut scene = Scene::new();
    let mut hud = HudInfo::collect(world);
    let mut seq = 0_u64;
    let mut accumulator = 0.0_f64;
    let mut last = Instant::now();
    let mut next_hud = last + HUD_PERIOD;
    // Publishing: when the last snapshot was made, of what (the tick and the request), what it cost, and the time slept since.
    let mut last_publish = last;
    let mut published: Option<(u64, SceneRequest)> = None;
    let mut cost = Duration::ZERO;
    let mut spare = Duration::ZERO;
    while !shared.stop.load(Ordering::Acquire) {
        let now = Instant::now();
        let dt = now.duration_since(last).as_secs_f64().min(0.25);
        last = now;
        let engine = f64::from(f32::from_bits(shared.engine.load(Ordering::Relaxed)));
        accumulator += dt * engine;
        // The ticks that are owed, one after the other. The window asking for the world ends the burst after the tick in
        // progress; a burst that runs out of time while it is still behind drops the rest: a world whose ticks take longer
        // than the time they are owed runs as fast as it can, and does not owe what it cannot run.
        let mut ticked = false;
        while accumulator >= fixed {
            world.tick();
            accumulator -= fixed;
            shared.ticks.fetch_add(1, Ordering::Relaxed);
            ticked = true;
            if shared.stop.load(Ordering::Acquire) {
                break;
            }
            if now.elapsed() >= BURST {
                if accumulator >= fixed {
                    accumulator = 0.0;
                }
                break;
            }
        }
        let now = Instant::now();
        if now >= next_hud {
            hud = HudInfo::collect(world);
            next_hud = now + HUD_PERIOD;
        }
        // A snapshot is made when something changed (a tick ran, or the window asked for something else) and it is paid for:
        // by the time the worker has slept, or, for a world that leaves it no time to sleep, by the seventh of its time.
        let since_publish = now.duration_since(last_publish);
        if since_publish >= PUBLISH_PERIOD {
            let request = *lock(&shared.request);
            let ticks = shared.ticks.load(Ordering::Relaxed);
            if let Some(request) = request
                && published != Some((ticks, request))
                && ((!ticked && spare >= cost)
                    || since_publish >= PUBLISH_PERIOD.max(cost * PUBLISH_COST_SHARE))
            {
                scene.tool_view = request.tool_view;
                build_scene(
                    &mut scene,
                    world,
                    &widened(request.camera),
                    request.selected,
                    &request.options,
                    assets,
                    None,
                );
                seq += 1;
                let snapshot = Arc::new(Snapshot {
                    seq,
                    scene: scene.published(),
                    sim_time: world.simulated_time(),
                    tick: world.clock.tick(),
                    hud: hud.clone(),
                    hover: request.hover.map(|point| hover_answer(world, point)),
                    pellets: world.pellets.len(),
                    bibites: world.bibites.len(),
                });
                *lock(&shared.latest) = Some(snapshot);
                let end = Instant::now();
                let took = end - now;
                cost = if cost.is_zero() {
                    took
                } else {
                    (cost + took) / 2
                };
                spare = spare.saturating_sub(took);
                last_publish = end;
                published = Some((ticks, request));
            }
        }
        if !ticked {
            // Ahead of the clock: sleep until the next tick or snapshot is due.
            let until_tick = if engine > 0.0 {
                Duration::from_secs_f64(((fixed - accumulator) / engine).clamp(0.0, 1.0))
            } else {
                IDLE_SLEEP_MAX
            };
            let until_publish = (last_publish + PUBLISH_PERIOD).saturating_duration_since(now);
            let slept = Instant::now();
            std::thread::sleep(
                until_tick
                    .min(until_publish)
                    .clamp(IDLE_SLEEP_MIN, IDLE_SLEEP_MAX),
            );
            spare = (spare + slept.elapsed()).min(SPARE_CAP);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LoadedWorld;
    use bibites_core::math::Vec2;
    use bibites_core::sim::state_fingerprint;

    fn assert_send<T: Send>() {}

    /// A small generated world; the same seed makes the same world.
    fn starter(seed: u64) -> World {
        LoadedWorld::load(None, seed, 24, 1)
            .expect("a starter world")
            .world
    }

    /// Waits (twenty seconds at the most) until `done` holds.
    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let started = Instant::now();
        while !done() {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn ready_thread() -> SimThread {
        let mut sim = SimThread::spawn(None);
        wait_for("the worker to start", || sim.is_ready());
        sim
    }

    fn window_camera(world: &World) -> Camera {
        Camera::fit(
            world.bounds.min(),
            world.bounds.max(),
            Vec2::new(1280.0, 800.0),
            24.0,
        )
    }

    fn request_for(camera: Camera) -> SceneRequest {
        SceneRequest {
            camera,
            selected: None,
            options: SceneOptions::default(),
            tool_view: ToolView::default(),
            hover: None,
        }
    }

    #[test]
    fn a_world_can_move_between_threads() {
        assert_send::<World>();
        assert_send::<Box<World>>();
        assert_send::<Snapshot>();
    }

    #[test]
    fn the_worker_ticks_the_world_as_the_loop_does() {
        let mut sim = ready_thread();
        // A speed far above what a tick takes: the worker ticks back to back.
        sim.set_engine(1_000.0);
        let world = starter(9_101);
        let first = world.clock.tick();
        sim.run(Box::new(world));
        assert!(sim.is_running());
        wait_for("forty ticks", || sim.ticks() >= 40);
        let world = sim.reclaim();
        assert!(!sim.is_running());
        let ticks = sim.ticks();
        assert_eq!(
            world.clock.tick(),
            first + ticks,
            "every tick the worker counted is in the world, and no other"
        );
        let mut inline = starter(9_101);
        inline.run(ticks);
        assert_eq!(
            state_fingerprint(&world),
            state_fingerprint(&inline),
            "the world ticked on the worker diverged from one ticked in the loop"
        );
    }

    #[test]
    fn the_world_goes_and_comes_back_again_and_again() {
        let mut sim = ready_thread();
        sim.set_engine(1_000.0);
        let mut world = Box::new(starter(9_102));
        let first = world.clock.tick();
        for round in 0..3 {
            sim.run(world);
            let target = sim.ticks() + 10;
            wait_for("ten more ticks", || sim.ticks() >= target);
            world = sim.reclaim();
            assert_eq!(world.clock.tick(), first + sim.ticks(), "round {round}");
        }
        let mut inline = starter(9_102);
        inline.run(sim.ticks());
        assert_eq!(state_fingerprint(&world), state_fingerprint(&inline));
    }

    #[test]
    fn a_worker_asked_for_no_time_leaves_the_world_alone() {
        let mut sim = ready_thread();
        sim.set_engine(0.0);
        sim.set_request(request_for(window_camera(&starter(9_103))));
        sim.run(Box::new(starter(9_103)));
        // It publishes, so it has been through its loop a few times.
        wait_for("a snapshot", || sim.latest().is_some());
        std::thread::sleep(Duration::from_millis(30));
        let world = sim.reclaim();
        assert_eq!(sim.ticks(), 0);
        assert_eq!(
            state_fingerprint(&world),
            state_fingerprint(&starter(9_103))
        );
    }

    #[test]
    fn the_worker_does_not_run_ahead_of_the_clock() {
        let mut sim = ready_thread();
        // Ten times real time: 300 ticks a second, so about 150 in half a second. Far fewer is a machine too busy to
        // say anything; far more would be a worker that ignores the speed.
        sim.set_engine(10.0);
        sim.run(Box::new(starter(9_104)));
        std::thread::sleep(Duration::from_millis(500));
        let _ = sim.reclaim();
        let ticks = sim.ticks();
        assert!(
            (20..=220).contains(&ticks),
            "{ticks} ticks in half a second at 10x"
        );
    }

    #[test]
    fn a_snapshot_is_what_an_inline_build_makes_for_the_wider_camera() {
        let mut sim = ready_thread();
        sim.set_engine(0.0);
        let world = starter(9_105);
        let camera = window_camera(&world);
        let request = request_for(camera);
        sim.set_request(request);
        sim.run(Box::new(starter(9_105)));
        wait_for("a snapshot", || sim.latest().is_some());
        let snapshot = sim.latest().expect("a snapshot");

        let mut expected = Scene::new();
        expected.tool_view = request.tool_view;
        build_scene(
            &mut expected,
            &world,
            &widened(camera),
            None,
            &request.options,
            None,
            None,
        );
        assert!(!expected.primitives.is_empty());
        assert_eq!(snapshot.scene.primitives, expected.primitives);
        assert_eq!(snapshot.scene.dots, expected.dots, "the dots the runs of the display list refer to travel with it");
        assert_eq!(snapshot.scene.background, expected.background);
        assert_eq!(snapshot.scene.tool_view, expected.tool_view);
        assert!(snapshot.scene.sort_scratch.is_empty());
        assert_eq!(snapshot.tick, world.clock.tick());
        assert_eq!(snapshot.sim_time, world.simulated_time());
        assert_eq!(snapshot.bibites, world.bibites.len());
        assert_eq!(snapshot.pellets, world.pellets.len());
        assert_eq!(snapshot.hud, HudInfo::collect(&world));

        // A new request is drawn from the next snapshot on: a camera zoomed in four times sees a part of the world.
        let mut zoomed = camera;
        zoomed.zoom_at(camera.screen * 0.5, 4.0);
        let mut expected = Scene::new();
        build_scene(
            &mut expected,
            &world,
            &widened(zoomed),
            None,
            &request.options,
            None,
            None,
        );
        assert_ne!(expected.primitives.len(), snapshot.scene.primitives.len());
        sim.set_request(request_for(zoomed));
        wait_for("a snapshot of the new camera", || {
            sim.latest()
                .is_some_and(|latest| latest.scene.primitives == expected.primitives)
        });

        // The layers the player switched off are not drawn from the next snapshot on.
        let options = SceneOptions {
            pellets: false,
            grid: false,
            ..request.options
        };
        let mut expected = Scene::new();
        build_scene(
            &mut expected,
            &world,
            &widened(zoomed),
            None,
            &options,
            None,
            None,
        );
        assert_ne!(
            expected.primitives.len(),
            sim.latest().expect("a snapshot").scene.primitives.len()
        );
        sim.set_request(SceneRequest {
            options,
            ..request_for(zoomed)
        });
        wait_for("a snapshot without pellets and grid", || {
            sim.latest()
                .is_some_and(|latest| latest.scene.primitives == expected.primitives)
        });
        let _ = sim.reclaim();
    }

    /// A spot of `world` away from every body, where a pellet can lie alone.
    fn empty_spot(world: &World) -> Vec2 {
        let mut spot = Vec2::new(30.0, 20.0);
        for _ in 0..200 {
            if crate::selection::entity_at(world, spot).is_none()
                && crate::selection::tool_at(world, spot).is_none()
            {
                break;
            }
            spot += Vec2::new(25.0, 0.0);
        }
        spot
    }

    #[test]
    fn the_thread_answers_for_the_pellet_under_the_pointer() {
        use bibites_core::pellet::{MaterialKind, PelletSpawn};
        let mut world = starter(9_109);
        let spot = empty_spot(&world);
        let id = world
            .spawn_pellet(PelletSpawn::new(MaterialKind::Plant, 40.0, spot))
            .expect("a pellet");
        let body = crate::tooltip::pellet_body(&world, id).expect("a body");
        // A bibite with a pellet under it: the bibite has the mouse-over.
        let (_, record) = world.bibites.iter().next().expect("a bibite");
        let on_bibite = record.pos;
        let under = world
            .spawn_pellet(PelletSpawn::new(MaterialKind::Plant, 40.0, on_bibite))
            .expect("a pellet");
        assert!(
            crate::selection::pellet_under(&world, on_bibite).is_some_and(|found| found == under)
        );

        let mut sim = ready_thread();
        sim.set_engine(0.0);
        let camera = window_camera(&world);
        let ask = |point| SceneRequest {
            hover: Some(point),
            ..request_for(camera)
        };
        sim.set_request(ask(spot));
        sim.run(Box::new(world));
        wait_for("the answer for the pellet", || {
            sim.latest()
                .and_then(|latest| latest.hover.clone())
                .is_some_and(|hover| hover.point == spot)
        });
        let hover = sim
            .latest()
            .and_then(|latest| latest.hover.clone())
            .expect("an answer");
        assert_eq!(hover.pellet, Some((id, body)));

        // Nothing there, and a bibite on top of a pellet: no pellet tooltip.
        let nothing = spot + Vec2::new(0.0, 400.0);
        for point in [nothing, on_bibite] {
            sim.set_request(ask(point));
            wait_for("the answer for another point", || {
                sim.latest()
                    .and_then(|latest| latest.hover.clone())
                    .is_some_and(|hover| hover.point == point)
            });
            let hover = sim
                .latest()
                .and_then(|latest| latest.hover.clone())
                .expect("an answer");
            assert!(hover.pellet.is_none(), "a pellet at {point:?}");
        }
        // No pointer: no answer.
        sim.set_request(request_for(camera));
        wait_for("a snapshot without a question", || {
            sim.latest().is_some_and(|latest| latest.hover.is_none())
        });
        let _ = sim.reclaim();
    }

    #[test]
    fn snapshots_follow_the_world_as_it_runs() {
        let mut sim = ready_thread();
        sim.set_engine(1_000.0);
        let world = starter(9_106);
        sim.set_request(request_for(window_camera(&world)));
        let first = world.clock.tick();
        sim.run(Box::new(world));
        // A world that ticks all the time still gets a snapshot after every period.
        let mut seen = Vec::new();
        wait_for("three snapshots", || {
            if let Some(latest) = sim.latest()
                && seen.last().is_none_or(|(seq, _)| *seq != latest.seq)
            {
                seen.push((latest.seq, latest.tick));
            }
            seen.len() >= 3
        });
        let latest = sim.latest().expect("a snapshot");
        let world = sim.reclaim();
        assert!(
            seen.windows(2).all(|pair| pair[0].1 < pair[1].1),
            "each snapshot is of a later tick: {seen:?}"
        );
        assert!(latest.tick > first);
        assert!(latest.tick <= world.clock.tick());
        assert!(latest.sim_time > 0.0);
        sim.clear_snapshot();
        assert!(sim.latest().is_none());
    }

    #[test]
    fn a_husk_has_the_settings_of_its_world_and_no_content() {
        let mut world = starter(9_107);
        world.settings.independents.simulation_size = 321.0;
        let husk = husk_like(&world);
        assert_eq!(husk.settings, world.settings);
        assert!(!world.bibites.is_empty() && !world.pellets.is_empty());
        assert_eq!(husk.bibites.len(), 0);
        assert_eq!(husk.pellets.len(), 0);
    }

    #[test]
    fn a_worker_that_panicked_is_noticed() {
        let sim = ready_thread();
        assert!(!sim.has_died());
        sim.make_it_panic();
        let started = Instant::now();
        while !sim.has_died() {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "the panic was not noticed"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        // Dropping a dead thread neither hangs nor panics again.
        drop(sim);
    }

    /// `cargo test --profile perf -p bibites-view --bins -- --ignored --nocapture worker_throughput`: how many ticks the
    /// worker runs a second, and how many snapshots it makes, at 1x, 3x and as fast as it can, with and without a window
    /// asking for snapshots (`RUSTIBITES_SIM_BENCH_BIBITES`, default 3,000; `RUSTIBITES_SIM_BENCH_THREADS`, default 12). The
    /// game assets are used when `assets/game` is there.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn worker_throughput() {
        let bibites = std::env::var("RUSTIBITES_SIM_BENCH_BIBITES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(3_000);
        let threads = std::env::var("RUSTIBITES_SIM_BENCH_THREADS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(12);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/game");
        let root = root.join("manifest.json").is_file().then_some(root);
        println!(
            "worker throughput: {bibites} bibites, {threads} threads, assets {}",
            if root.is_some() { "yes" } else { "no" }
        );
        let mut sim = SimThread::spawn(root);
        wait_for("the worker to start", || sim.is_ready());
        let mut world = Box::new(
            LoadedWorld::load(None, 4_242, bibites, threads)
                .expect("a starter world")
                .world,
        );
        let request = request_for(window_camera(&world));
        let seconds = 4.0;
        let mut measure =
            |label: &str, engine: f32, publish: bool, world: Box<World>| -> Box<World> {
                sim.set_engine(engine);
                if publish {
                    sim.set_request(request);
                } else {
                    sim.clear_request();
                }
                sim.clear_snapshot();
                let before = sim.ticks();
                sim.run(world);
                std::thread::sleep(Duration::from_secs_f64(seconds));
                let snapshots = sim.latest().map_or(0, |latest| latest.seq);
                let world = sim.reclaim();
                let ticks = sim.ticks() - before;
                println!(
                    "  {label}: {:6.1} ticks/s, {:5.1} snapshots/s",
                    ticks as f64 / seconds,
                    snapshots as f64 / seconds,
                );
                world
            };
        world = measure("warm up                ", 10_000.0, false, world);
        for (name, engine) in [("1x", 1.0), ("3x", 3.0), ("flat out", 10_000.0)] {
            world = measure(&format!("{name:8} no snapshots  "), engine, false, world);
            world = measure(&format!("{name:8} with snapshots"), engine, true, world);
        }
        drop(world);
    }

    #[test]
    fn an_idle_worker_publishes_as_often_as_the_world_changes() {
        let mut sim = ready_thread();
        let world = starter(9_110);
        sim.set_request(request_for(window_camera(&world)));
        // Nothing changes at speed zero: one snapshot, and no more however long the worker looks at the clock.
        sim.set_engine(0.0);
        sim.run(Box::new(world));
        wait_for("the first snapshot", || sim.latest().is_some());
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(sim.latest().expect("a snapshot").seq, 1);
        let world = sim.reclaim();

        // A world that ticks all the time and has time to spare: a snapshot for nearly every period.
        sim.set_engine(10.0);
        sim.clear_snapshot();
        sim.run(world);
        std::thread::sleep(Duration::from_millis(600));
        let snapshots = sim.latest().map_or(0, |latest| latest.seq);
        let _ = sim.reclaim();
        assert!(
            snapshots >= 12,
            "{snapshots} snapshots in 0.6 s of a cheap world that ticks 300 times a second"
        );
    }

    #[test]
    fn dropping_the_thread_while_it_runs_ends_it() {
        let mut sim = ready_thread();
        sim.set_engine(1_000.0);
        sim.run(Box::new(starter(9_108)));
        wait_for("a tick", || sim.ticks() >= 1);
        // `Drop` joins the worker: a hang here is a worker that does not look at the stop flag.
        drop(sim);
    }
}
