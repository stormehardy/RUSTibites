//! A frame's ticks on a helper thread while the window draws and presents that frame.
//!
//! A frame of the window is: input, the world's ticks, the display list and the UI built from the world, the drawing (GPU
//! commands) and the presenting, which waits for the display (the frame's vsync, a GPU that is not done yet). One thread did all of
//! it in that order, so everything after the world had been read (drawing, presenting, waiting) came out of the time the ticks could
//! have had: a bigger window, a view of a busier part of the world or a slower GPU made the simulation slower, and the CPU
//! sat idle in the wait (the testers: "the speed depends on where the camera is", "on the screen resolution; the smaller the window,
//! the faster it runs and the more CPU it uses"). The minimum-FPS rule gives a frame `1 / min_fps` seconds and the ticks got what the
//! rest of the frame left.
//!
//! Now the ticks of a frame run on this thread from the moment the display list and the UI are built (the last readers of the world
//! in the frame) until the frame has been presented: the window moves the world here for that time and takes it back right after
//! `next_frame`, before anything reads it again. The ticks are the world's own, one after the other, as before; only WHEN they run
//! changes (what is drawn is the world as it was before the frame's ticks, one frame of ticks behind the simulation).

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::Instant;

use bibites_core::world::World;

/// One frame's ticks.
pub struct Batch {
    /// The real world, for the time of the batch.
    pub world: Box<World>,
    /// The ticks owed. The first one always runs; more only while the next is expected to fit `room`.
    pub owed: u32,
    /// The seconds the ticks of this frame may take (`None`: no limit, the cap on `owed` is the only one).
    pub room: Option<f64>,
    /// What a tick is expected to cost, in seconds (an average of the recent ones; 0 before the first).
    pub tick_cost: f64,
    /// Tests: every tick counts as this many seconds (a model machine) instead of the time it took.
    pub model_tick_secs: Option<f64>,
}

/// What a batch did.
pub struct Finished {
    /// The world, after the ticks.
    pub world: Box<World>,
    /// The ticks that ran.
    pub ticks: u32,
    /// Seconds they took.
    pub spent: f64,
    /// The average cost of a tick after these ticks.
    pub tick_cost: f64,
}

/// Runs the ticks of `batch`: the loop of the frame pacing (one tick when one is owed, more while the next fits the room), the
/// average cost of a tick kept as `0.7 old + 0.3 new`.
#[must_use]
pub fn run_batch(batch: Batch) -> Finished {
    let Batch {
        mut world,
        owed,
        room,
        mut tick_cost,
        model_tick_secs,
    } = batch;
    let mut spent = 0.0_f64;
    let mut ticks = 0_u32;
    while ticks < owed {
        if ticks >= 1 && room.is_some_and(|room| spent + tick_cost > room) {
            break;
        }
        let started = Instant::now();
        world.tick();
        let took = model_tick_secs.unwrap_or_else(|| started.elapsed().as_secs_f64());
        ticks += 1;
        spent += took;
        tick_cost = if tick_cost == 0.0 {
            took
        } else {
            0.7 * tick_cost + 0.3 * took
        };
    }
    Finished {
        world,
        ticks,
        spent,
        tick_cost,
    }
}

/// A tick that panicked: the payload, to be raised again on the window's thread (the world went with it).
type Outcome = Result<Finished, Box<dyn Any + Send>>;

/// The helper thread: it runs one batch at a time and hands the world back.
pub struct TickWorker {
    jobs: Option<Sender<Batch>>,
    outcomes: Receiver<Outcome>,
    handle: Option<JoinHandle<()>>,
    busy: bool,
}

impl TickWorker {
    /// Starts the thread (it sleeps until a batch comes).
    #[must_use]
    pub fn spawn() -> Self {
        let (jobs, job_rx) = channel::<Batch>();
        let (outcome_tx, outcomes) = channel::<Outcome>();
        let handle = std::thread::Builder::new()
            .name("tick-overlap".to_owned())
            // A tick of a big world runs on this thread when the pool has one thread.
            .stack_size(16 << 20)
            .spawn(move || worker(job_rx, outcome_tx))
            .expect("the tick thread starts");
        Self {
            jobs: Some(jobs),
            outcomes,
            handle: Some(handle),
            busy: false,
        }
    }

    /// Hands the world to the thread for the ticks of `batch`. A batch must not be out.
    pub fn start(&mut self, batch: Batch) {
        assert!(!self.busy, "a batch of ticks is already running");
        self.busy = true;
        self.jobs
            .as_ref()
            .expect("the tick thread is alive")
            .send(batch)
            .expect("the tick thread is alive");
    }

    /// Waits for the batch that is out and takes the world back. A tick that panicked on the thread panics here, with its message.
    pub fn finish(&mut self) -> Finished {
        assert!(self.busy, "no batch of ticks is running");
        self.busy = false;
        match self
            .outcomes
            .recv()
            .expect("the tick thread ended without an answer")
        {
            Ok(finished) => finished,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}

impl Drop for TickWorker {
    fn drop(&mut self) {
        // Closing the channel ends the thread's loop (after the batch it may be in).
        self.jobs = None;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn worker(jobs: Receiver<Batch>, outcomes: Sender<Outcome>) {
    while let Ok(batch) = jobs.recv() {
        let outcome = catch_unwind(AssertUnwindSafe(|| run_batch(batch)));
        if outcomes.send(outcome).is_err() {
            return;
        }
    }
}
