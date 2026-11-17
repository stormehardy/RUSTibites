//! `OneUseScripts.TimeKeeper` (`Here/OneUseScripts/TimeKeeper.cs`) and the time panel's text (`GUIManager.UpdateTime`):
//! the frames per second, the ticks per second and the sim/real factor the game shows are **measured**, not the settings.
//!
//! Every [`UPDATE_INTERVAL`] seconds of real time the game divides the rendered frames it counted by the time they took
//! (`fps`), and the simulation ticks it ran by the time they took (`tps`), and `stps = tps * Time.fixedDeltaTime` is the
//! number of simulated seconds that passed per real second (the "sim/real" line). Nothing is measured while the game is
//! paused: the three numbers keep the values of the last measurement (a world that was just loaded and paused still reads
//! `0 tps` and `sim/real: x0.00`), and the first frame and the first tick after a pause only restart the counting.
//!
//! The same file holds the game's minimum-FPS rules (`TimeController.CheckMinFPS` and `SetTarget`): the time slider sets a
//! *target* time scale, the scale the simulation really runs at (the *engine* scale) follows it, and when a measurement says the
//! frame rate fell under the player's minimum the engine scale is lowered in proportion, so that the window stays usable and
//! "sim/real" shows how much slower than asked the simulation ran.

/// `TimeKeeper.updateInterval`: seconds of real time between two measurements.
pub const UPDATE_INTERVAL: f64 = 0.5;

/// The measured speed of the viewer: what the game's `TimeKeeper` keeps.
#[derive(Clone, Debug, Default)]
pub struct TimeKeeper {
    /// `fps`: rendered frames per second over the last window.
    fps: f32,
    /// `tps`: simulation ticks per second over the last window.
    tps: f32,
    /// `stps`: simulated seconds per real second (`tps * fixedDeltaTime`).
    stps: f32,
    /// `renderFrames`.
    render_frames: u32,
    /// `lastRenderInterval`: real time the render window started.
    last_render_interval: f64,
    /// `fixedFrames`.
    fixed_frames: u32,
    /// `lastFixedInterval`: real time the tick window started.
    last_fixed_interval: f64,
    /// `wasPaused`.
    was_paused: bool,
    /// `wasFixedPaused`.
    was_fixed_paused: bool,
    /// `firstUpdate`: no measurement of the frame rate has been made yet (the first one does not adjust the engine scale).
    first_update: bool,
}

impl TimeKeeper {
    /// `Start()`: both windows begin at real time `now` (seconds), with nothing counted and every number zero.
    #[must_use]
    pub fn new(now: f64) -> Self {
        Self {
            last_render_interval: now,
            last_fixed_interval: now,
            first_update: true,
            ..Self::default()
        }
    }

    /// `Update()`: one rendered frame at real time `now`; `paused` is `TimeController.paused`.
    ///
    /// Returns the frame rate when this frame closed a window of measurement that the game then hands to `CheckMinFPS` (every
    /// one but the first), for the caller to apply [`min_fps_scale`] with.
    pub fn render_frame(&mut self, now: f64, paused: bool) -> Option<f32> {
        if paused {
            self.was_paused = true;
            self.was_fixed_paused = true;
            return None;
        }
        if self.was_paused {
            // The first frame after a pause only restarts the window.
            self.last_render_interval = now;
            self.render_frames = 0;
            self.was_paused = false;
            return None;
        }
        self.render_frames += 1;
        if now > self.last_render_interval + UPDATE_INTERVAL {
            self.fps = (f64::from(self.render_frames) / (now - self.last_render_interval)) as f32;
            self.render_frames = 0;
            self.last_render_interval = now;
            let check = !self.first_update;
            self.first_update = false;
            return check.then_some(self.fps);
        }
        None
    }

    /// `TimeKeeper.firstUpdate`: no frame rate has been measured yet.
    #[must_use]
    pub fn first_update(&self) -> bool {
        self.first_update
    }

    /// `FixedUpdate()`: one simulation tick of `fixed_dt` simulated seconds (`Time.fixedDeltaTime`) at real time `now`.
    pub fn fixed_tick(&mut self, now: f64, fixed_dt: f32) {
        if self.was_fixed_paused {
            // The first tick after a pause only restarts the window.
            self.fixed_frames = 0;
            self.last_fixed_interval = now;
            self.was_fixed_paused = false;
            return;
        }
        self.fixed_frames += 1;
        if now > self.last_fixed_interval + UPDATE_INTERVAL {
            self.tps = (f64::from(self.fixed_frames) / (now - self.last_fixed_interval)) as f32;
            self.stps = self.tps * fixed_dt;
            self.fixed_frames = 0;
            self.last_fixed_interval = now;
        }
    }

    /// `renderFramesPerSecond`.
    #[must_use]
    pub fn fps(&self) -> f32 {
        self.fps
    }

    /// `logicFramePerSeconds`: simulation ticks per real second.
    #[must_use]
    pub fn tps(&self) -> f32 {
        self.tps
    }

    /// `simSecondsPerSeconds`: simulated seconds per real second.
    #[must_use]
    pub fn sim_seconds_per_second(&self) -> f32 {
        self.stps
    }
}

/// `TimeController.CheckMinFPS`: the engine time scale after a measurement of `present_fps` (`None`: the game leaves it as it is).
///
/// With a minimum of at least 1 frame per second, not paused, and the measurement further from the minimum than
/// `log2(min) - 2` frames per second, the engine scale becomes `engine * present_fps / min_fps` (lower when the frame rate is
/// under the minimum, higher when it is over, 4 times at 60 against 15), kept between `min(0.1, target)` and the target.
/// (The game skips this while its window is unfocused when "Ignore FPS minimum When Unfocused" is ticked; the window's focus is
/// not known here, so it is taken as focused.)
#[must_use]
pub fn min_fps_scale(
    engine: f32,
    target: f32,
    present_fps: f32,
    min_fps: u32,
    paused: bool,
) -> Option<f32> {
    if min_fps < 1 || paused {
        return None;
    }
    let min = min_fps as f32;
    if (min - present_fps).abs() < min.log2() - 2.0 {
        return None;
    }
    let value = engine * present_fps / min;
    if value.is_nan() {
        return None;
    }
    // `Mathf.Clamp(value, Mathf.Min(0.1f, target), target)`.
    let lower = target.min(0.1);
    Some(if value < lower {
        lower
    } else if value > target {
        target
    } else {
        value
    })
}

/// `TimeController.SetTarget`: the engine time scale after the time slider was moved to `target`.
///
/// A target under the engine scale takes over at once; so does any target with the minimum FPS switched off; otherwise the
/// frame-rate rule is asked with the last measurement (once there is one), and the engine scale stays where it is when it has
/// nothing to say: the governor raises it towards the target over the next measurements.
#[must_use]
pub fn engine_scale_after_target(
    engine: f32,
    target: f32,
    min_fps: u32,
    first_update: bool,
    present_fps: f32,
    paused: bool,
) -> f32 {
    if target < engine || min_fps < 1 {
        return target;
    }
    if !first_update
        && let Some(value) = min_fps_scale(engine, target, present_fps, min_fps, paused)
    {
        return value;
    }
    engine
}

/// `TimeKeeper.ParseTime`: `[hours, minutes, seconds]` of a simulated time (the hours do not wrap at a day).
#[must_use]
pub fn parse_time(time: f64) -> [i32; 3] {
    let whole = (time as f32).floor() as i32;
    let seconds = whole % 60;
    let total_minutes = (whole - seconds) / 60;
    let minutes = total_minutes % 60;
    let hours = (total_minutes - minutes) / 60;
    let seconds = ((time % 60.0) as f32).floor() as i32;
    [hours, minutes, seconds]
}

/// C# `value.ToString("F0")` of a non-negative float: rounded half away from zero.
fn f0(value: f32) -> String {
    format!("{}", f64::from(value).round() as i64)
}

/// `GUIManager.UpdateTime`: the three lines of the time panel,
/// `Time: HH:MM:SS`, `{fps:F0} fps` (left-aligned in 6 characters) `  /  {tps:F0} tps`, and `sim/real: x{stps:F2}`.
#[must_use]
pub fn time_panel_text(simulated_time: f64, keeper: &TimeKeeper) -> String {
    let [hours, minutes, seconds] = parse_time(simulated_time);
    format!(
        "Time: {hours:02}:{minutes:02}:{seconds:02}\n{:<6}  /  {} tps\nsim/real: x{:.2}",
        format!("{} fps", f0(keeper.fps())),
        f0(keeper.tps()),
        keeper.sim_seconds_per_second()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `seconds` of real time in frames of `1 / frames_per_second`, with `ticks_per_second` ticks spread evenly over them
    /// (as the viewer's accumulator does), and returns the real time reached.
    fn run(
        keeper: &mut TimeKeeper,
        mut now: f64,
        seconds: f64,
        frames_per_second: f64,
        ticks_per_second: f64,
        fixed_dt: f32,
    ) -> f64 {
        let frame = 1.0 / frames_per_second;
        let end = now + seconds;
        let mut owed = 0.0;
        while now < end - 1e-9 {
            now += frame;
            owed += ticks_per_second * frame;
            while owed >= 1.0 {
                keeper.fixed_tick(now, fixed_dt);
                owed -= 1.0;
            }
            keeper.render_frame(now, false);
        }
        now
    }

    #[test]
    fn everything_reads_zero_until_a_window_has_passed() {
        let mut keeper = TimeKeeper::new(0.0);
        assert_eq!(
            time_panel_text(0.0, &keeper),
            "Time: 00:00:00\n0 fps   /  0 tps\nsim/real: x0.00"
        );
        // 0.4 s of running: no window has closed yet.
        run(&mut keeper, 0.0, 0.4, 60.0, 30.0, 1.0 / 30.0);
        assert_eq!(keeper.fps(), 0.0);
        assert_eq!(keeper.tps(), 0.0);
        assert_eq!(keeper.sim_seconds_per_second(), 0.0);
    }

    #[test]
    fn a_steady_run_reads_the_frames_and_ticks_it_really_made() {
        let mut keeper = TimeKeeper::new(0.0);
        run(&mut keeper, 0.0, 3.0, 60.0, 30.0, 1.0 / 30.0);
        assert!((keeper.fps() - 60.0).abs() < 2.0, "fps {}", keeper.fps());
        assert!((keeper.tps() - 30.0).abs() < 2.0, "tps {}", keeper.tps());
        assert!(
            (keeper.sim_seconds_per_second() - 1.0).abs() < 0.07,
            "sim/real {}",
            keeper.sim_seconds_per_second()
        );
    }

    #[test]
    fn a_world_that_cannot_keep_up_shows_the_rate_it_manages_not_the_one_asked_for() {
        // Asked for 90 ticks per second (speed 3 of 30), the machine manages 61 at 10 frames per second.
        let mut keeper = TimeKeeper::new(0.0);
        run(&mut keeper, 0.0, 3.0, 10.0, 61.0, 1.0 / 30.0);
        assert!((keeper.fps() - 10.0).abs() < 1.5, "fps {}", keeper.fps());
        assert!((keeper.tps() - 61.0).abs() < 4.0, "tps {}", keeper.tps());
        assert!(
            (keeper.sim_seconds_per_second() - 61.0 / 30.0).abs() < 0.15,
            "sim/real {}",
            keeper.sim_seconds_per_second()
        );
    }

    #[test]
    fn a_pause_keeps_the_last_numbers_and_the_first_frame_and_tick_after_it_only_restart_the_windows()
     {
        let mut keeper = TimeKeeper::new(0.0);
        let mut now = run(&mut keeper, 0.0, 2.0, 60.0, 30.0, 1.0 / 30.0);
        let (fps, tps, stps) = (keeper.fps(), keeper.tps(), keeper.sim_seconds_per_second());
        assert!(tps > 28.0 && fps > 58.0);
        // Five seconds paused: only frames, no ticks, and nothing changes (the numbers are not zeroed).
        for _ in 0..300 {
            now += 1.0 / 60.0;
            keeper.render_frame(now, true);
        }
        assert_eq!(
            (keeper.fps(), keeper.tps(), keeper.sim_seconds_per_second()),
            (fps, tps, stps)
        );
        // Resumed: the pause is not in the window, so the rate does not dip towards zero when it closes.
        now = run(&mut keeper, now, 1.0, 60.0, 30.0, 1.0 / 30.0);
        assert!((keeper.tps() - 30.0).abs() < 2.0, "tps {}", keeper.tps());
        assert!((keeper.fps() - 60.0).abs() < 2.0, "fps {}", keeper.fps());
        let _ = now;
    }

    #[test]
    fn a_world_loaded_paused_keeps_reading_zero() {
        // The reference capture of a freshly loaded, paused world: 0 tps, sim/real x0.00.
        let mut keeper = TimeKeeper::new(0.0);
        let mut now = 0.0;
        for _ in 0..120 {
            now += 1.0 / 60.0;
            keeper.render_frame(now, true);
        }
        assert_eq!(keeper.tps(), 0.0);
        assert_eq!(keeper.sim_seconds_per_second(), 0.0);
        assert!(time_panel_text(20.0, &keeper).ends_with("0 tps\nsim/real: x0.00"));
    }

    #[test]
    fn the_panel_text_is_the_games() {
        let mut keeper = TimeKeeper::new(0.0);
        run(&mut keeper, 0.0, 2.0, 144.0, 30.0, 1.0 / 30.0);
        let text = time_panel_text(3725.9, &keeper);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Time: 01:02:05");
        // Three digits of fps fill the 6 characters of the alignment: `{fps,-6}` then two spaces, the slash, two spaces.
        assert!(
            lines[1].starts_with("144 fps  /  ") || lines[1].starts_with("143 fps  /  "),
            "{}",
            lines[1]
        );
        assert!(lines[1].ends_with(" tps"), "{}", lines[1]);
        assert!(lines[2].starts_with("sim/real: x1.0"), "{}", lines[2]);
        // Fewer digits are padded to 6 characters before the two spaces.
        let mut slow = TimeKeeper::new(0.0);
        run(&mut slow, 0.0, 3.0, 4.0, 30.0, 1.0 / 30.0);
        assert!(
            time_panel_text(0.0, &slow)
                .lines()
                .nth(1)
                .unwrap()
                .starts_with("4 fps   /  "),
            "{}",
            time_panel_text(0.0, &slow)
        );
    }

    #[test]
    fn simulated_time_is_shown_in_whole_seconds_and_the_hours_do_not_wrap() {
        assert_eq!(parse_time(0.0), [0, 0, 0]);
        assert_eq!(parse_time(59.99), [0, 0, 59]);
        assert_eq!(parse_time(60.0), [0, 1, 0]);
        assert_eq!(parse_time(3725.9), [1, 2, 5]);
        assert_eq!(parse_time(100.0 * 3600.0 + 61.5), [100, 1, 1]);
    }

    #[test]
    fn the_first_measurement_does_not_adjust_the_engine_scale_and_the_next_ones_do() {
        let mut keeper = TimeKeeper::new(0.0);
        assert!(keeper.first_update());
        let mut checks = Vec::new();
        let mut now = 0.0;
        for _ in 0..180 {
            now += 1.0 / 60.0;
            if let Some(fps) = keeper.render_frame(now, false) {
                checks.push(fps);
            }
        }
        // 3 seconds: windows close about every half second, the first closes without a check.
        assert!(!keeper.first_update());
        assert!(checks.len() >= 4 && checks.len() <= 5, "{checks:?}");
        assert!(
            checks.iter().all(|fps| (fps - 60.0).abs() < 2.0),
            "{checks:?}"
        );
        // Paused frames never report one.
        assert_eq!(keeper.render_frame(now + 1.0, true), None);
    }

    #[test]
    fn a_frame_rate_under_the_minimum_lowers_the_engine_scale_in_proportion() {
        // 5 fps against a minimum of 15: a third of the scale.
        let lowered = min_fps_scale(6.0, 6.0, 5.0, 15, false).unwrap();
        assert!((lowered - 2.0).abs() < 1e-5, "{lowered}");
        // Again: 5 fps still: a third again, and it can go down to 0.1 and no further.
        let again = min_fps_scale(lowered, 6.0, 5.0, 15, false).unwrap();
        assert!((again - 2.0 / 3.0).abs() < 1e-5, "{again}");
        assert_eq!(min_fps_scale(0.15, 6.0, 1.0, 15, false), Some(0.1));
        // Over the minimum it climbs back, 60 fps against 15 is four times, and never past the target.
        let climbed = min_fps_scale(0.5, 6.0, 60.0, 15, false).unwrap();
        assert!((climbed - 2.0).abs() < 1e-5, "{climbed}");
        assert_eq!(min_fps_scale(2.0, 3.0, 60.0, 15, false), Some(3.0));
        // The lower bound is min(0.1, target): a target of 0.05 (the slider goes to 0.1, a loaded one need not) holds.
        assert_eq!(min_fps_scale(0.05, 0.05, 1.0, 15, false), Some(0.05));
    }

    #[test]
    fn the_engine_scale_is_left_alone_near_the_minimum_when_paused_and_with_the_minimum_off() {
        // log2(15) - 2 = 1.9 frames per second of dead band.
        assert_eq!(min_fps_scale(3.0, 3.0, 14.0, 15, false), None);
        assert_eq!(min_fps_scale(3.0, 3.0, 16.5, 15, false), None);
        assert!(min_fps_scale(3.0, 3.0, 12.0, 15, false).is_some());
        assert!(min_fps_scale(3.0, 3.0, 18.0, 15, false).is_some());
        // Paused, or a minimum of 0: the game does nothing.
        assert_eq!(min_fps_scale(3.0, 3.0, 5.0, 15, true), None);
        assert_eq!(min_fps_scale(3.0, 3.0, 5.0, 0, false), None);
        // A minimum of 1 has a negative dead band: always adjusts.
        assert!(min_fps_scale(3.0, 3.0, 1.0, 1, false).is_some());
        // A frame rate that is not a number changes nothing.
        assert_eq!(min_fps_scale(3.0, 3.0, f32::NAN, 15, false), None);
    }

    #[test]
    fn moving_the_time_slider_sets_or_asks_for_the_engine_scale_as_the_game_does() {
        // A target under the engine scale takes over at once.
        assert_eq!(
            engine_scale_after_target(4.0, 2.0, 15, false, 5.0, false),
            2.0
        );
        // With the minimum off any target does.
        assert_eq!(
            engine_scale_after_target(1.0, 5.0, 0, false, 5.0, false),
            5.0
        );
        // Raised with a measurement at hand: the frame-rate rule decides (60 fps: four times the scale, here under the target).
        let raised = engine_scale_after_target(1.0, 10.0, 15, false, 60.0, false);
        assert!((raised - 4.0).abs() < 1e-5, "{raised}");
        // Raised before the first measurement, or while paused (the rule does nothing): the engine scale stays.
        assert_eq!(
            engine_scale_after_target(1.0, 5.0, 15, true, 60.0, false),
            1.0
        );
        assert_eq!(
            engine_scale_after_target(1.0, 5.0, 15, false, 60.0, true),
            1.0
        );
    }

    #[test]
    fn f0_rounds_half_away_from_zero_like_the_game() {
        assert_eq!(f0(29.5), "30");
        assert_eq!(f0(30.49), "30");
        assert_eq!(f0(0.5), "1");
        assert_eq!(f0(2.5), "3");
        assert_eq!(f0(0.0), "0");
    }
}
