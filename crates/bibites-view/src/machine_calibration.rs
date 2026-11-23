//! The calibration of the machine, as the Settings screens' Calibration page shows it: the test runs on a thread of its own ([`Calibrator`]), what
//! it found is kept in a file beside the settings and handed to the simulation (`bibites_core::calibration`, `bibites_core::sim::MachineProfile`), and a
//! test of the graphics (what the window's GPU shader and the processor's way of drawing the bibites cost on this machine) runs in the window after it.
//!
//! The page's rows (a check box, a button and four lines of text) are made in `settings_pages.rs` / `improved_settings.rs`; this module owns what they say
//! ([`Lines`]) and what the button does ([`start`], [`cancel`]). Nothing here changes what a world computes: the numbers only tell the simulation how to share its
//! work out and the window which way of drawing is the faster.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use bibites_core::calibration::{self, Calibration, Config, Progress, Stop};
use bibites_core::sim::{self, MachineProfile};

/// The label of the check box row of the page (the setting that says whether the stored calibration is used).
pub const LABEL_USE: &str = "Use Calibration";
/// The file the calibration is kept in, in the folder of the settings.
const FILE: &str = "calibration.txt";
/// The longest line the page's text rows show (the capture's rows hold about this many characters).
pub const LINE_CHARS: usize = 56;
/// The graphics test's lines in the file.
const GRAPHICS_KEY: &str = "graphics";

/// What the page's help text says for a row of it (title and text), by label.
#[must_use]
pub fn help_of(label: &str) -> Option<(&'static str, &'static str)> {
    if label.trim().eq_ignore_ascii_case(LABEL_USE) {
        Some((
            LABEL_USE,
            "If enabled, the simulation uses what the calibration test found about this machine: how many workers a tick uses and from what size of world the pool of workers pays (the first guess a world starts from). If not, it starts from the sizes measured on the developer's machine. The results of the simulation are the same either way: only the speed changes. Run the test with the button below; it takes about half a minute and the result is kept.",
        ))
    } else {
        None
    }
}

/// What a machine has, as Windows tells (asked once, on a thread: it takes a second).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hardware {
    /// The processor's name.
    pub cpu: String,
    /// Physical cores.
    pub cores: usize,
    /// Logical threads.
    pub threads: usize,
    /// The names of the graphics adapters.
    pub gpus: Vec<String>,
}

impl Hardware {
    /// Reads what `powershell` says about the processor and the graphics adapters (nothing where there is none: the fields stay empty).
    #[must_use]
    pub fn query() -> Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            const SCRIPT: &str = "$p = @(Get-CimInstance Win32_Processor); \
                $c = ($p | Measure-Object NumberOfCores -Sum).Sum; \
                $t = ($p | Measure-Object NumberOfLogicalProcessors -Sum).Sum; \
                $g = @(Get-CimInstance Win32_VideoController | ForEach-Object { $_.Name }); \
                [pscustomobject]@{ cpu = $p[0].Name; cores = $c; threads = $t; gpus = $g } | ConvertTo-Json -Compress";
            let output = std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
                .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                .output();
            if let Ok(output) = output
                && output.status.success()
            {
                return Self::from_json(&String::from_utf8_lossy(&output.stdout));
            }
        }
        Self { threads: calibration::available_threads(), ..Self::default() }
    }

    /// Reads the answer of the query (a JSON object; a missing or odd field stays empty).
    #[must_use]
    pub fn from_json(text: &str) -> Self {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
            return Self::default();
        };
        let number = |key: &str| value.get(key).and_then(serde_json::Value::as_u64).unwrap_or(0) as usize;
        let gpus = match value.get("gpus") {
            Some(serde_json::Value::Array(names)) => names.iter().filter_map(|name| name.as_str()).map(str::to_owned).collect(),
            Some(serde_json::Value::String(name)) => vec![name.clone()],
            _ => Vec::new(),
        };
        Self { cpu: value.get("cpu").and_then(serde_json::Value::as_str).unwrap_or("").trim().to_owned(), cores: number("cores"), threads: number("threads"), gpus }
    }

    /// The graphics adapter the program most likely draws with: the first that is not the software one Windows keeps for when there is no driver.
    #[must_use]
    pub fn main_gpu(&self) -> Option<&str> {
        self.gpus.iter().map(String::as_str).find(|name| !name.to_ascii_lowercase().contains("basic render")).or_else(|| self.gpus.first().map(String::as_str))
    }
}

/// A name without what clutters a line: the trademark signs, "Processor", "CPU", the clock and the vendor.
#[must_use]
pub fn short_name(name: &str) -> String {
    let mut text = name.replace("(R)", "").replace("(TM)", "").replace("(tm)", "");
    for word in ["Processor", "CPU", "Graphics", "with Radeon", "NVIDIA ", "AMD ", "Intel "] {
        text = text.replace(word, "");
    }
    // "@ 3.60GHz" and the like.
    if let Some(at) = text.find('@') {
        text.truncate(at);
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A line cut to what a row of the page holds (three dots where it was cut: the game's font has no ellipsis sign).
#[must_use]
pub fn fit_line(text: &str) -> String {
    if text.chars().count() <= LINE_CHARS {
        return text.to_owned();
    }
    let kept: String = text.chars().take(LINE_CHARS - 3).collect();
    format!("{}...", kept.trim_end())
}

/// What the graphics test measured: the cost of drawing a scene of bibites with the GPU shader and with the processor's way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Graphics {
    /// Sprites (and other primitives) of the scene that was drawn.
    pub primitives: usize,
    /// Milliseconds to build the scene (the processor, one thread).
    pub scene_ms: f32,
    /// Milliseconds to draw it with the GPU hue shader (`None`: the shader did not load on this machine).
    pub gpu_ms: Option<f32>,
    /// Milliseconds to draw it with the sprites recoloured on the processor (`None`: not measured).
    pub cpu_ms: Option<f32>,
}

impl Graphics {
    /// Whether the window should try the GPU shader for the bibites: yes, unless the shader was measured at least half as slow again as the processor's way. (Both
    /// ways cost mostly the same per sprite, the drawing calls, so a card with a driver measures within a tenth either way, and the test's own noise is that big; a
    /// machine with no graphics driver runs the shader in software and measures several times slower. Without a measurement of the shader the answer is yes too: a
    /// shader the driver does not take falls back by itself, at start, and a test run with `--no-gpu-hue-shader` has none.)
    #[must_use]
    pub fn prefers_gpu_shader(&self) -> bool {
        match (self.gpu_ms, self.cpu_ms) {
            (Some(gpu), Some(cpu)) => gpu <= cpu * 1.5,
            _ => true,
        }
    }

    fn to_text(self) -> String {
        let number = |value: Option<f32>| value.map_or_else(|| "-".to_owned(), |ms| format!("{ms:.3}"));
        format!("{GRAPHICS_KEY}={},{:.3},{},{}\n", self.primitives, self.scene_ms, number(self.gpu_ms), number(self.cpu_ms))
    }

    fn from_text(text: &str) -> Option<Self> {
        let line = text.lines().find_map(|line| line.strip_prefix(GRAPHICS_KEY).and_then(|rest| rest.strip_prefix('=')))?;
        let parts: Vec<&str> = line.trim().split(',').collect();
        let [primitives, scene, gpu, cpu] = parts[..] else {
            return None;
        };
        let optional = |part: &str| if part == "-" { Some(None) } else { part.parse::<f32>().ok().map(Some) };
        Some(Self { primitives: primitives.parse().ok()?, scene_ms: scene.parse().ok()?, gpu_ms: optional(gpu)?, cpu_ms: optional(cpu)? })
    }
}

/// The run that is going on.
struct Running {
    progress: Progress,
    cancel: Arc<AtomicBool>,
}

/// What the page knows (behind the lock of a [`Calibrator`]).
#[derive(Default)]
struct Model {
    hardware: Option<Hardware>,
    hardware_asked: bool,
    running: Option<Running>,
    result: Option<Calibration>,
    graphics: Option<Graphics>,
    /// A stored calibration was found, but it is for a machine with another number of threads.
    stale: bool,
    /// What the last run or the last click said (an error, or why nothing started).
    note: String,
    /// A new result or setting that the simulation has not been given yet.
    changed: bool,
    /// The CPU part just finished: the window runs the graphics test next.
    graphics_due: bool,
    use_calibration: bool,
    world_running: bool,
}

/// The test on a thread of its own, and what the page says about it. One of them lives for the whole process ([`calibrator`]); the tests make their own.
pub struct Calibrator {
    model: Arc<Mutex<Model>>,
}

impl Default for Calibrator {
    fn default() -> Self {
        Self::new()
    }
}

/// The lines of the page, as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lines {
    /// What the button says.
    pub button: String,
    /// How far the test is, or what it found, or what it needs.
    pub status: String,
    /// The processor, and the cores and threads it has.
    pub machine: String,
    /// What the test found about the pool of workers.
    pub multicore: String,
    /// The graphics adapter and what the graphics test found.
    pub graphics: String,
}

impl Lines {
    /// Every line in one string, to see whether the page has to be painted again.
    #[must_use]
    pub fn signature(&self) -> String {
        format!("{}|{}|{}|{}|{}", self.button, self.status, self.machine, self.multicore, self.graphics)
    }
}

impl Calibrator {
    /// A calibrator with nothing run and nothing stored.
    #[must_use]
    pub fn new() -> Self {
        Self { model: Arc::new(Mutex::new(Model { use_calibration: true, ..Model::default() })) }
    }

    fn lock(&self) -> MutexGuard<'_, Model> {
        self.model.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Reads the stored calibration in `root` (the settings' folder). A stored one for a machine with another number of threads is kept out and said so.
    pub fn load(&self, root: &Path) {
        let Ok(text) = std::fs::read_to_string(file_in(root)) else {
            return;
        };
        let mut model = self.lock();
        match Calibration::from_text(&text) {
            Ok(found) if found.is_for_this_machine() => {
                model.graphics = Graphics::from_text(&text);
                model.result = Some(found);
                model.stale = false;
            }
            Ok(_) => model.stale = true,
            Err(_) => {}
        }
        model.changed = true;
    }

    /// Whether the stored calibration is used (the check box).
    pub fn set_use(&self, on: bool) {
        let mut model = self.lock();
        if model.use_calibration != on {
            model.use_calibration = on;
            model.changed = true;
        }
    }

    /// Whether a world is running in the window (a test would measure it as much as the machine).
    pub fn set_world_running(&self, running: bool) {
        self.lock().world_running = running;
    }

    /// The profile the simulation should use now: the calibration's, when there is one and the check box is on.
    #[must_use]
    pub fn profile(&self) -> Option<MachineProfile> {
        let model = self.lock();
        if model.use_calibration { model.result.as_ref().map(Calibration::profile) } else { None }
    }

    /// Whether the profile changed since the last call (a result, the check box, a stored file read): the window gives it to the simulation.
    pub fn take_changed(&self) -> bool {
        std::mem::take(&mut self.lock().changed)
    }

    /// Whether the graphics test is due (the processor's part finished); asking takes the turn.
    pub fn take_graphics_due(&self) -> bool {
        std::mem::take(&mut self.lock().graphics_due)
    }

    /// Whether a test is going on.
    #[must_use]
    pub fn running(&self) -> bool {
        self.lock().running.is_some()
    }

    /// Keeps what the graphics test found, in the file in `root` after the calibration's own lines.
    pub fn set_graphics(&self, graphics: Graphics, root: &Path) {
        let mut model = self.lock();
        model.graphics = Some(graphics);
        if let Some(result) = &model.result {
            write_file(root, result, Some(graphics));
        }
    }

    /// The stored graphics result: whether the bibites are drawn with the GPU shader by default.
    #[must_use]
    pub fn graphics(&self) -> Option<Graphics> {
        self.lock().graphics
    }

    /// Asks Windows about the processor and the graphics adapters, once, on a thread (the page's lines say "looking" until it has answered).
    pub fn ask_hardware(&self) {
        {
            let mut model = self.lock();
            if model.hardware_asked {
                return;
            }
            model.hardware_asked = true;
        }
        let model = Arc::clone(&self.model);
        let _ = std::thread::Builder::new().name("hardware".to_owned()).spawn(move || {
            let hardware = Hardware::query();
            model.lock().unwrap_or_else(std::sync::PoisonError::into_inner).hardware = Some(hardware);
        });
    }

    /// Starts the test on a thread of its own with `config`, keeping the result in `root`. Returns what is wrong when it cannot (it is also the page's line).
    ///
    /// # Errors
    /// A test is already going on, or a world is running in the window.
    pub fn start(&self, config: Config, root: PathBuf) -> Result<(), String> {
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut model = self.lock();
            if model.running.is_some() {
                return Err("A test is already going on".to_owned());
            }
            if model.world_running {
                model.note = "Pause the world first (or run it from the title screen)".to_owned();
                return Err(model.note.clone());
            }
            model.note.clear();
            model.running = Some(Running { progress: Progress { done: 0, total: 0, what: "Starting".to_owned() }, cancel: Arc::clone(&cancel) });
        }
        self.ask_hardware();
        let model = Arc::clone(&self.model);
        let spawned = std::thread::Builder::new().name("calibration".to_owned()).spawn(move || {
            let outcome = calibration::run(&config, &cancel, &mut |progress| {
                if let Some(running) = model.lock().unwrap_or_else(std::sync::PoisonError::into_inner).running.as_mut() {
                    running.progress = progress.clone();
                }
            });
            let mut model = model.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            model.running = None;
            match outcome {
                Ok(found) => {
                    write_file(&root, &found, None);
                    model.result = Some(found);
                    model.graphics = None;
                    model.stale = false;
                    model.changed = true;
                    model.graphics_due = true;
                }
                Err(Stop::Cancelled) => model.note = "Stopped: nothing was changed".to_owned(),
                Err(Stop::NoWorld) => model.note = "The test could not build its worlds".to_owned(),
            }
        });
        if let Err(error) = spawned {
            let mut model = self.lock();
            model.running = None;
            model.note = format!("The test could not start: {error}");
            return Err(model.note.clone());
        }
        Ok(())
    }

    /// Asks the test to stop at its next block.
    pub fn cancel(&self) {
        if let Some(running) = self.lock().running.as_ref() {
            running.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The page's text now.
    #[must_use]
    pub fn lines(&self) -> Lines {
        let model = self.lock();
        let threads = calibration::available_threads();
        let hardware = model.hardware.as_ref();
        let machine = match hardware {
            None => format!("Looking... ({threads} threads)"),
            Some(hardware) => {
                let cpu = short_name(&hardware.cpu);
                let shape = if hardware.cores > 0 && hardware.threads > 0 {
                    format!("{} cores, {} threads", hardware.cores, hardware.threads)
                } else {
                    format!("{threads} threads")
                };
                if cpu.is_empty() { shape } else { format!("{cpu}: {shape}") }
            }
        };
        let status = if let Some(running) = &model.running {
            if running.progress.total == 0 { running.progress.what.clone() } else { format!("{}/{} {}", running.progress.done + 1, running.progress.total, running.progress.what) }
        } else if !model.note.is_empty() {
            model.note.clone()
        } else if let Some(result) = &model.result {
            let busy = if result.noisy() { "; the machine was busy: run it again" } else { "" };
            format!("Done in {:.0} s{busy}", result.seconds)
        } else if model.stale {
            "The stored test is for another machine: run it again".to_owned()
        } else {
            "Not run yet".to_owned()
        };
        let multicore = match &model.result {
            None => "Not measured".to_owned(),
            Some(result) if result.rows.is_empty() => "One thread: every world runs on it".to_owned(),
            Some(result) => {
                let used = if model.use_calibration { "" } else { " (not used)" };
                format!("{} workers, from {} bibites, up to {:.1}x{used}", result.workers, result.pool_bibites, result.speedup)
            }
        };
        let adapter = hardware.and_then(|hardware| hardware.main_gpu()).map(short_name).filter(|name| !name.is_empty());
        let graphics = match (adapter, model.graphics) {
            (adapter, Some(graphics)) => {
                let way = |ms: Option<f32>, name: &str| ms.map(|ms| format!("{name} {ms:.1} ms"));
                let costs = [way(graphics.gpu_ms, "GPU"), way(graphics.cpu_ms, "CPU")].into_iter().flatten().collect::<Vec<_>>().join(", ");
                let costs = if costs.is_empty() { "no GPU shader here".to_owned() } else { costs };
                match adapter {
                    Some(adapter) => format!("{adapter}: {costs}"),
                    None => costs,
                }
            }
            (Some(adapter), None) => format!("{adapter} (not tested yet)"),
            (None, None) => if hardware.is_some() { "Not tested yet".to_owned() } else { "Looking...".to_owned() },
        };
        let button = if model.running.is_some() { "Stop" } else { "Run Test" }.to_owned();
        Lines { button, status: fit_line(&status), machine: fit_line(&machine), multicore: fit_line(&multicore), graphics: fit_line(&graphics) }
    }
}

/// The path of the calibration file in the settings' folder.
#[must_use]
pub fn file_in(root: &Path) -> PathBuf {
    root.join(FILE)
}

fn write_file(root: &Path, result: &Calibration, graphics: Option<Graphics>) {
    let mut text = result.to_text();
    if let Some(graphics) = graphics {
        text.push_str(&graphics.to_text());
    }
    // Written beside, then moved over the old file, so a stop in the middle leaves the one that was there.
    let (path, temporary) = (file_in(root), root.join(format!("{FILE}.tmp")));
    if std::fs::create_dir_all(root).is_ok() && std::fs::write(&temporary, text).is_ok() && std::fs::rename(&temporary, &path).is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
}

/// The one calibrator of the process.
pub fn calibrator() -> &'static Calibrator {
    static CALIBRATOR: OnceLock<Calibrator> = OnceLock::new();
    CALIBRATOR.get_or_init(Calibrator::new)
}

/// The settings' folder (where the file is kept).
#[must_use]
pub fn default_root() -> PathBuf {
    bibites_io::storage::StoragePaths::default_local().root
}

/// Reads the stored calibration and gives it to the process: at the start of the program, before any world is made.
pub fn init(use_calibration: bool) {
    let calibrator = calibrator();
    calibrator.set_use(use_calibration);
    calibrator.load(&default_root());
    apply(calibrator);
}

/// Gives the process the calibrator's profile (or takes it away, when the check box is off or nothing was measured).
pub fn apply(calibrator: &Calibrator) {
    sim::set_machine_profile(calibrator.profile());
}

/// What the Calibration button does: starts the test (the full one), or stops the one that is going on.
pub fn button_pressed() {
    let calibrator = calibrator();
    if calibrator.running() {
        calibrator.cancel();
    } else {
        // (The reason it did not start is on the page's status line.)
        let _ = calibrator.start(Config::full(calibration::available_threads()), default_root());
    }
}

/// The live text of a node of the page, by its name (`None`: not a node of this page).
#[must_use]
pub fn live_text(node_name: &str, node_path: &str) -> Option<String> {
    let calibrator = calibrator();
    let is = |name: &str| node_name == name;
    if node_path.ends_with(&format!("/{}/Text (TMP)", names::RUN)) {
        calibrator.ask_hardware();
        return Some(calibrator.lines().button);
    }
    if !(is(names::STATUS) || is(names::MACHINE) || is(names::MULTICORE) || is(names::GRAPHICS)) {
        return None;
    }
    calibrator.ask_hardware();
    let lines = calibrator.lines();
    Some(match node_name {
        names::STATUS => lines.status,
        names::MACHINE => lines.machine,
        names::MULTICORE => lines.multicore,
        _ => lines.graphics,
    })
}

/// The page's text as one string (to paint the page again when it changes).
#[must_use]
pub fn page_signature() -> String {
    let calibrator = calibrator();
    calibrator.ask_hardware();
    calibrator.lines().signature()
}

/// Times how the window draws a crowd of bibites on this machine: the scene is built once, then drawn into a picture of its own (not the window: nothing flickers)
/// with the GPU hue shader and, when the game's art is there, with the sprites recoloured on the processor; each way is timed by the picture being read back, which
/// waits for the graphics card to be done. A second at most, on the window's thread (the card's context lives there); the page says "Looking" or "not tested" until
/// it is done. `None`: no game art to draw with, or the picture could not be made.
#[must_use]
pub fn graphics_test(
    textures: &[macroquad::prelude::Texture2D],
    hue: Option<&crate::render::GpuHueMaterial>,
    zone: Option<&crate::render::GpuZoneMaterial>,
    assets: Option<&crate::assets::GameAssets>,
) -> Option<Graphics> {
    use macroquad::camera::{Camera2D, set_camera, set_default_camera};
    use macroquad::prelude::{Rect, render_target};
    /// The size of the picture the crowd is drawn into, and the crowd.
    const WIDTH: u32 = 1280;
    const HEIGHT: u32 = 720;
    const CROWD: usize = 1_200;
    /// Draws of the scene to a timing, timings of a way (the first is not counted: the caches and uploads of a way are warmed by it).
    const DRAWS: usize = 6;
    const TIMINGS: usize = 5;

    let assets = assets?;
    if textures.is_empty() {
        return None;
    }
    // A packed world, seen from close: the sprites are big enough to be sprites (not the dots of the zoomed-out view).
    let world = calibration::synthetic_world_in(CROWD, 0x6A11, 120.0).ok()?;
    let screen = bibites_core::math::Vec2::new(WIDTH as f32, HEIGHT as f32);
    let camera = crate::render::Camera::new(bibites_core::math::Vec2::ZERO, HEIGHT as f32 / 60.0, screen);
    let mut scene = crate::render::Scene::default();
    let options = crate::render::SceneOptions { grid: false, ..crate::render::SceneOptions::default() };
    let built = Instant::now();
    crate::render::build_scene(&mut scene, &world, &camera, None, &options, Some(assets), None);
    let scene_ms = built.elapsed().as_secs_f32() * 1000.0;
    let primitives = scene.primitives.len();

    let target = render_target(WIDTH, HEIGHT);
    let drawing = Camera2D { render_target: Some(target.clone()), ..Camera2D::from_display_rect(Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32)) };
    // What reading a picture back costs by itself (the same for every way): a cleared picture, the same wait.
    let time_way = |draw: &dyn Fn()| -> f32 {
        let mut best = f32::MAX;
        for timing in 0..TIMINGS {
            let started = Instant::now();
            set_camera(&drawing);
            draw();
            set_default_camera();
            let _ = target.texture.get_texture_data();
            if timing > 0 {
                best = best.min(started.elapsed().as_secs_f32() * 1000.0);
            }
        }
        best
    };
    let nothing = time_way(&|| macroquad::prelude::clear_background(macroquad::prelude::BLACK));
    let atlases = &assets.atlases[..];
    let per_draw = |hue: Option<&crate::render::GpuHueMaterial>| -> f32 {
        let total = time_way(&|| {
            for _ in 0..DRAWS {
                crate::render::draw_scene(&scene, &camera, textures, hue, zone, atlases);
            }
        });
        ((total - nothing) / DRAWS as f32).max(0.01)
    };
    let gpu_ms = hue.map(|material| per_draw(Some(material)));
    let cpu_ms = Some(per_draw(None));
    Some(Graphics { primitives, scene_ms, gpu_ms, cpu_ms })
}

/// `RUSTIBITES_CALIBRATE=tiny|quick|full`: the test runs as soon as the window is up (in the frames' own way: the test of the graphics follows the processor's part),
/// what the page would say is printed, and the program ends. For checking the test in the real window without clicking; called once a frame. Returns whether to end.
pub fn autorun_finished() -> bool {
    use std::sync::atomic::AtomicU32;
    static MODE: OnceLock<Option<String>> = OnceLock::new();
    static STATE: AtomicU32 = AtomicU32::new(0);
    let Some(mode) = MODE.get_or_init(|| std::env::var("RUSTIBITES_CALIBRATE").ok()).as_deref() else {
        return false;
    };
    let calibrator = calibrator();
    let state = STATE.fetch_add(1, Ordering::Relaxed);
    if state == 0 {
        let threads = calibration::available_threads();
        let config = match mode {
            "tiny" => Config::tiny(threads),
            "quick" => Config::quick(threads),
            _ => Config::full(threads),
        };
        if let Err(why) = calibrator.start(config, default_root()) {
            eprintln!("calibration: {why}");
            return true;
        }
        return false;
    }
    // Done when the processor's part has, and the graphics test has had its turn (a few frames: it runs in the frame after the processor's part ends).
    static WAITED: AtomicU32 = AtomicU32::new(0);
    let finished = !calibrator.running() && (calibrator.graphics().is_some() || WAITED.fetch_add(1, Ordering::Relaxed) >= 30);
    if !finished {
        return false;
    }
    let lines = calibrator.lines();
    eprintln!("calibration: status   {}", lines.status);
    eprintln!("calibration: machine  {}", lines.machine);
    eprintln!("calibration: multicore {}", lines.multicore);
    eprintln!("calibration: graphics {}", lines.graphics);
    if let Some(result) = calibrator.lock().result.as_ref() {
        for line in result.to_text().lines() {
            eprintln!("calibration: | {line}");
        }
    }
    true
}

/// The names of the page's nodes (the menu's hit code and live text look for them).
pub mod names {
    /// The name of the button that runs the test (its text node says "Run Test" or "Stop").
    pub const RUN: &str = "CalibrationRun";
    /// The line that says how far the test is.
    pub const STATUS: &str = "CalibrationStatus";
    /// The line that says what the machine is.
    pub const MACHINE: &str = "CalibrationMachine";
    /// The line that says what the pool of workers found.
    pub const MULTICORE: &str = "CalibrationMultiCore";
    /// The line that says what the graphics test found.
    pub const GRAPHICS: &str = "CalibrationGraphics";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("rustibites-calibration-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn wait_until_done(calibrator: &Calibrator) {
        let deadline = Instant::now() + std::time::Duration::from_secs(60);
        while calibrator.running() {
            assert!(Instant::now() < deadline, "the tiny test took over a minute");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn the_answer_of_the_hardware_query_is_read_whatever_shape_a_single_adapter_comes_in() {
        let two = Hardware::from_json(r#"{"cpu":"AMD Ryzen 5 3600 6-Core Processor             ","cores":6,"threads":12,"gpus":["Microsoft Basic Render Driver","NVIDIA GeForce GTX 1660"]}"#);
        assert_eq!((two.cores, two.threads), (6, 12));
        assert_eq!(two.cpu, "AMD Ryzen 5 3600 6-Core Processor");
        assert_eq!(two.main_gpu(), Some("NVIDIA GeForce GTX 1660"), "not the software adapter");
        // PowerShell writes a list of one as a bare string in some versions.
        let one = Hardware::from_json(r#"{"cpu":"Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz","cores":8,"threads":8,"gpus":"Intel(R) UHD Graphics 630"}"#);
        assert_eq!(one.gpus, ["Intel(R) UHD Graphics 630"]);
        let basic_only = Hardware { gpus: vec!["Microsoft Basic Render Driver".to_owned()], ..Hardware::default() };
        assert_eq!(basic_only.main_gpu(), Some("Microsoft Basic Render Driver"));
        assert_eq!(Hardware::from_json("not json"), Hardware::default());
        assert_eq!(Hardware::default().main_gpu(), None);
    }

    #[test]
    fn names_lose_what_clutters_a_line_and_lines_are_cut_to_the_rows_width() {
        assert_eq!(short_name("AMD Ryzen 5 3600 6-Core Processor"), "Ryzen 5 3600 6-Core");
        assert_eq!(short_name("Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz"), "Core i7-9700K");
        assert_eq!(short_name("NVIDIA GeForce GTX 1660"), "GeForce GTX 1660");
        assert_eq!(short_name("AMD Radeon(TM) Graphics"), "Radeon");
        let long = "x".repeat(80);
        let cut = fit_line(&long);
        assert_eq!(cut.chars().count(), LINE_CHARS);
        assert!(cut.ends_with("..."));
        assert_eq!(fit_line("short"), "short");
    }

    #[test]
    fn the_graphics_result_is_kept_in_the_file_and_decides_the_shader() {
        let measured = Graphics { primitives: 1_500, scene_ms: 0.9, gpu_ms: Some(0.4), cpu_ms: Some(3.2) };
        assert_eq!(Graphics::from_text(&measured.to_text()), Some(measured));
        let no_shader = Graphics { gpu_ms: None, ..measured };
        assert_eq!(Graphics::from_text(&no_shader.to_text()), Some(no_shader));
        assert_eq!(Graphics::from_text("threads=4\n"), None);
        assert_eq!(Graphics::from_text("graphics=1,2,3\n"), None, "a line of the wrong shape says nothing");
        assert!(measured.prefers_gpu_shader());
        // A software renderer: the GPU way is several times slower than the processor's.
        assert!(!Graphics { gpu_ms: Some(9.0), cpu_ms: Some(3.0), ..measured }.prefers_gpu_shader());
        // What a real card measured in the window (GTX 1660 SUPER: 3.9 against 3.5 ms, 4.4 against 3.7): noise, the GPU stays.
        assert!(Graphics { gpu_ms: Some(3.9), cpu_ms: Some(3.5), ..measured }.prefers_gpu_shader());
        assert!(Graphics { gpu_ms: Some(4.4), cpu_ms: Some(3.7), ..measured }.prefers_gpu_shader());
        assert!(Graphics { cpu_ms: None, ..measured }.prefers_gpu_shader());
        assert!(no_shader.prefers_gpu_shader(), "no measurement of the shader: it is tried at start and falls back by itself");
    }

    #[test]
    fn a_tiny_test_runs_on_its_thread_keeps_its_file_and_hands_over_a_profile_once() {
        let root = temp_root("run");
        let calibrator = Calibrator::new();
        assert_eq!(calibrator.profile(), None, "nothing run: no profile");
        assert!(!calibrator.take_changed());
        calibrator.start(Config::tiny(3), root.clone()).expect("it starts");
        assert_eq!(calibrator.start(Config::tiny(3), root.clone()), Err("A test is already going on".to_owned()), "one at a time");
        assert_eq!(calibrator.lines().button, "Stop");
        wait_until_done(&calibrator);
        assert_eq!(calibrator.lines().button, "Run Test");
        assert!(calibrator.lines().status.starts_with("Done in "), "{:?}", calibrator.lines());
        let profile = calibrator.profile().expect("a profile now");
        assert!((2..=3).contains(&profile.workers));
        assert!(calibrator.take_changed() && !calibrator.take_changed(), "the simulation is told once");
        assert!(calibrator.take_graphics_due() && !calibrator.take_graphics_due(), "and the window is asked for its test once");
        assert!(file_in(&root).is_file());
        // The check box: off takes the profile away without forgetting the result.
        calibrator.set_use(false);
        assert_eq!(calibrator.profile(), None);
        assert!(calibrator.take_changed());
        assert!(calibrator.lines().multicore.contains("(not used)"), "{:?}", calibrator.lines().multicore);
        calibrator.set_use(true);
        assert_eq!(calibrator.profile(), Some(profile));
        // A new calibrator reads the file back (for the machine the numbers were made on only: the tiny test claimed three threads).
        let again = Calibrator::new();
        again.load(&root);
        if calibration::available_threads() == 3 {
            assert_eq!(again.profile(), Some(profile));
        } else {
            assert_eq!(again.profile(), None, "a file for another machine is not used");
            assert!(again.lines().status.contains("another machine"), "{:?}", again.lines().status);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_test_that_is_stopped_changes_nothing_and_a_running_world_keeps_it_from_starting() {
        let root = temp_root("stop");
        let calibrator = Calibrator::new();
        calibrator.set_world_running(true);
        let refused = calibrator.start(Config::tiny(2), root.clone());
        assert!(refused.as_ref().is_err_and(|why| why.contains("Pause the world")), "{refused:?}");
        assert!(calibrator.lines().status.contains("Pause the world"));
        assert!(!calibrator.running());
        calibrator.set_world_running(false);
        // A bigger test than the tiny one, so there is time to stop it.
        let slow = Config { sizes: vec![1_000, 2_000, 3_000], rounds: 3, ..Config::tiny(2) };
        calibrator.start(slow, root.clone()).expect("it starts");
        calibrator.cancel();
        wait_until_done(&calibrator);
        assert_eq!(calibrator.profile(), None);
        assert!(!calibrator.take_changed());
        assert!(calibrator.lines().status.contains("Stopped"), "{:?}", calibrator.lines().status);
        assert!(!file_in(&root).exists(), "nothing was written");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_lines_say_what_is_known_and_none_is_longer_than_a_row() {
        let calibrator = Calibrator::new();
        let blank = calibrator.lines();
        assert_eq!(blank.status, "Not run yet");
        assert_eq!(blank.multicore, "Not measured");
        assert!(blank.machine.starts_with("Looking"), "{}", blank.machine);
        // With the hardware known and a result in.
        {
            let mut model = calibrator.lock();
            model.hardware = Some(Hardware { cpu: "AMD Ryzen 5 3600 6-Core Processor".to_owned(), cores: 6, threads: 12, gpus: vec!["NVIDIA GeForce GTX 1660".to_owned()] });
            model.result = Some(Calibration { threads: 12, workers: 12, pool_bibites: 880, start_level: 1, speedup: 2.34, noise: 0.5, seconds: 29.6, when: 1, worker_rows: vec![], rows: vec![calibration::SizeRow { bibites: 1, bodies: 1, ns: [1.0; 5] }] });
            model.graphics = Some(Graphics { primitives: 1_000, scene_ms: 0.8, gpu_ms: Some(0.46), cpu_ms: Some(3.21) });
        }
        let lines = calibrator.lines();
        assert_eq!(lines.machine, "Ryzen 5 3600 6-Core: 6 cores, 12 threads");
        assert_eq!(lines.multicore, "12 workers, from 880 bibites, up to 2.3x");
        assert_eq!(lines.status, "Done in 30 s; the machine was busy: run it again");
        assert_eq!(lines.graphics, "GeForce GTX 1660: GPU 0.5 ms, CPU 3.2 ms");
        for line in [&lines.button, &lines.status, &lines.machine, &lines.multicore, &lines.graphics] {
            assert!(line.chars().count() <= LINE_CHARS, "{line}");
        }
        // The signature changes when a line does.
        let before = lines.signature();
        calibrator.lock().graphics = None;
        assert_ne!(before, calibrator.lines().signature());
        assert_eq!(calibrator.lines().graphics, "GeForce GTX 1660 (not tested yet)");
    }

    #[test]
    fn the_checkbox_has_a_help_text_and_the_page_names_are_the_ones_the_menu_looks_for() {
        assert!(help_of(LABEL_USE).is_some_and(|(title, text)| title == "Use Calibration" && text.contains("same either way")));
        assert!(help_of("Streams").is_none());
        let all = [names::RUN, names::STATUS, names::MACHINE, names::MULTICORE, names::GRAPHICS];
        assert_eq!(all.iter().collect::<std::collections::BTreeSet<_>>().len(), all.len(), "no two nodes share a name");
    }
}
