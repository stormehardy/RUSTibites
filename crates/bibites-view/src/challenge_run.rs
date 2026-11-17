//! A challenge run as the game starts and plays it.
//!
//! `ScenarioSelectorPanel.StartChallenge` puts the picked champion into the scenario's bibite rows (`championSettings`, tagged
//! `Champion`) and starts the scene; `ChallengeManager.Start` then runs the challenge on that world: the exit condition and the
//! stars are looked at every second of scaled time, the end pauses the game ("ChallengeEnd") and shows `ChallengeEndPopup`.
//! `ChallengeTrackerPanel.RetryChallenge` starts the same scenario again (`GameManager.StartGame()`); the popup's two buttons are
//! `ReturnToMenu` and `ContinueAsScenario`.

use std::path::PathBuf;

use bibites_core::settings::ChallengeParameters;

use crate::app::{App, LoadedWorld};

/// The bibite a challenge is played with (`ScenarioSelectorPanel.selectedChampion`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Champion {
    /// The template file (`BibiteTemplate.fullPath`).
    pub path: PathBuf,
    /// The template's name (`championSettings.templateName`).
    pub name: String,
}

/// What the title menu asks for when it starts a challenge scenario (`StartChallenge(asChallenge)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChallengeStart {
    /// The scenario's `.zip`.
    pub scenario: PathBuf,
    /// The picked champion: added to the scenario's rows whatever `as_challenge` says.
    pub champion: Option<Champion>,
    /// `true`: "Start Challenge" (the challenge runs); `false`: the scenario is started as a scenario (`challengeParameters = null`).
    pub as_challenge: bool,
}

/// What a running challenge was made from; kept so that "Retry" can build the same world again.
#[derive(Clone, Debug)]
pub struct ChallengeSetup {
    /// The scenario's `.zip`.
    pub scenario: PathBuf,
    /// `scenario.info` name (`challengeName`).
    pub name: String,
    /// `scenario.info` description (`challengeDesc`).
    pub description: String,
    /// `scenario.info` star texts (`star1Desc`, `star2Desc`, `star3Desc`).
    pub stars: [String; 3],
    /// The scenario's challenge with the champion row filled in.
    pub parameters: ChallengeParameters,
    /// The champion.
    pub champion: Champion,
}

impl App {
    /// `ChallengeManager.Start` for a world that was built from a challenge: the session, the star texts, the restrictions
    /// (`challenge_is_running`), the tracker closed, no popup.
    pub fn begin_challenge(&mut self, setup: ChallengeSetup) {
        let started = self.world.simulated_time();
        let mut session = bibites_core::world::ChallengeSession::new(
            setup.parameters.clone(),
            setup.name.clone(),
            setup.description.clone(),
            setup.champion.name.clone(),
            setup.champion.path.display().to_string(),
            started,
        );
        session.star_descriptions = setup.stars.clone();
        self.challenge = Some(session);
        self.challenge_params = Some(setup.parameters.clone());
        self.challenge_setup = Some(setup);
        self.challenge_end_shown = false;
        self.challenge_end_pause = None;
        self.challenge_record = None;
        self.challenge_high = None;
        // `tracker.gameObject.SetActive(false)`: the tracker starts closed, the tab opens it.
        self.panels.close(crate::ui::PanelKind::ChallengeTracker);
    }

    /// `ChallengeTrackerPanel.RetryChallenge` (`GameManager.StartGame()`): the same scenario with the same champion in a new
    /// world. Nothing happens for a run that is not a challenge.
    pub fn restart_challenge(&mut self) {
        let Some(setup) = self.challenge_setup.clone() else {
            return;
        };
        let start = ChallengeStart {
            scenario: setup.scenario,
            champion: Some(setup.champion),
            as_challenge: true,
        };
        // Another seed than the run before: a retry that played out the same way would not be one.
        let seed = self
            .world
            .seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        match LoadedWorld::load_challenge_start(&start, seed, self.world.threads()) {
            Ok((loaded, Some(setup))) => {
                self.adopt_world(loaded);
                self.begin_challenge(setup);
                self.paused = false;
                self.accumulator = 0.0;
                self.last_action = "challenge restarted".to_owned();
            }
            Ok((_, None)) => {
                self.last_action = "challenge restart: the scenario is not a challenge".to_owned();
            }
            Err(error) => self.last_action = format!("challenge restart failed: {error}"),
        }
    }

    /// `ChallengeEndPopup.ContinueAsScenario` / `ChallengeManager.ContinueAsScenario`: the challenge is over and the world goes
    /// on as an ordinary scenario (the hidden controls are back, the end pause is released).
    pub fn continue_as_scenario(&mut self) {
        self.challenge = None;
        self.challenge_params = None;
        self.challenge_setup = None;
        self.challenge_end_shown = false;
        // `Object.Destroy(tracker.gameObject)`.
        self.panels.close(crate::ui::PanelKind::ChallengeTracker);
        if let Some(previous) = self.challenge_end_pause.take() {
            self.paused = previous;
            self.accumulator = 0.0;
        }
        self.last_action = "challenge closed".to_owned();
    }

    /// `ChallengeEndPopup.ReturnToMenu` / `ChallengeManager.ReturnToMenu`.
    pub fn challenge_return_to_menu(&mut self) {
        self.challenge = None;
        self.challenge_params = None;
        self.challenge_setup = None;
        self.challenge_end_shown = false;
        self.challenge_end_pause = None;
        self.panels.close(crate::ui::PanelKind::ChallengeTracker);
        self.return_to_menu = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{ControlCommand, UiAction};
    use bibites_core::math::Vec2;
    use bibites_core::settings::{
        BibiteSpawn, Comparator, ConditionGroup, SimCondition, SimMetric, SpawnType,
    };
    use std::path::Path;

    fn repo_file(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join(relative)
    }

    /// A challenge scenario made from the local `Default.zip` and a champion template, in a throwaway folder
    /// (`None` on a machine without the local test data).
    fn fixture(tag: &str) -> Option<(PathBuf, Champion, PathBuf)> {
        let default = repo_file("testdata/real/Default.zip");
        let template =
            repo_file("testdata/real/templates_current/Basic_bibite_0.6.3.1.bb8template");
        if !default.is_file() || !template.is_file() {
            return None;
        }
        let dir =
            std::env::temp_dir().join(format!("rustibites-challenge-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut bundle = bibites_io::load_scenario_bundle(&default).expect("Default.zip");
        bundle.info.name = "Little Rabbits".to_owned();
        bundle.info.description = "Grow from the initial 3.".to_owned();
        bundle.info.is_challenge = true;
        bundle.info.stars = [
            Some("Reach 5 population".to_owned()),
            Some("Reach 10 population".to_owned()),
            Some("Reach 20 population".to_owned()),
        ];
        let mut parameters = ChallengeParameters::game_defaults();
        parameters.exit_condition = ConditionGroup::single(SimCondition::new(
            SimMetric::Time,
            "",
            Comparator::GreaterThanOrEqual,
            SimMetric::Constant,
            "30",
        ));
        parameters.champion = BibiteSpawn {
            minimum: 3,
            spawn_priority: 1.0,
            spawn_type: SpawnType::OneTime,
            ..BibiteSpawn::champion_defaults()
        };
        bundle.settings_file.challenge = Some(parameters);
        let scenario = dir.join("Little Rabbits.zip");
        bibites_io::write_scenario_bundle(&bundle, &scenario).expect("write the scenario");
        let champion = Champion {
            path: template,
            name: "Basic bibite".to_owned(),
        };
        Some((scenario, champion, dir))
    }

    fn start(scenario: &Path, champion: Option<&Champion>, as_challenge: bool) -> ChallengeStart {
        ChallengeStart {
            scenario: scenario.to_path_buf(),
            champion: champion.cloned(),
            as_challenge,
        }
    }

    /// `TagCount("Champion")`: the metric the challenge scores with.
    fn champions(app: &App) -> usize {
        app.world.metric_value(SimMetric::TagCount, "Champion") as usize
    }

    #[test]
    fn a_challenge_starts_with_the_champion_row_of_the_scenario_and_no_extra_champion() {
        let Some((scenario, champion, dir)) = fixture("row") else {
            return;
        };
        let (loaded, setup) =
            LoadedWorld::load_challenge_start(&start(&scenario, Some(&champion), true), 7, 1)
                .expect("the challenge loads");
        // The row is the game's: tagged `Champion`, `minimum` of them, spawned by the spawner (not one more at the camera).
        let row = loaded
            .world
            .bibite_spawns
            .iter()
            .find(|row| row.tag == "Champion")
            .expect("the champion's row is in the scenario's rows");
        assert_eq!(row.minimum, 3);
        assert_eq!(row.tagging, bibites_core::settings::Tagging::CustomTagging);
        let setup = setup.expect("started as a challenge");
        assert_eq!(setup.name, "Little Rabbits");
        assert_eq!(setup.stars[1], "Reach 10 population");
        assert_eq!(setup.champion, champion);
        let mut app = App::new(loaded, Vec2::new(1280.0, 800.0), None);
        for _ in 0..600 {
            app.world.tick();
        }
        assert_eq!(champions(&app), 3, "exactly the row's minimum");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn begin_challenge_makes_the_session_the_tracker_reads() {
        let Some((scenario, champion, dir)) = fixture("session") else {
            return;
        };
        let (loaded, setup) =
            LoadedWorld::load_challenge_start(&start(&scenario, Some(&champion), true), 8, 1)
                .expect("loads");
        let mut app = App::new(loaded, Vec2::new(1280.0, 800.0), None);
        assert!(
            !app.challenge_is_running(),
            "a world is not a challenge until it begins one"
        );
        app.begin_challenge(setup.expect("setup"));
        let session = app.challenge.as_ref().expect("the session");
        assert_eq!(session.challenge_name, "Little Rabbits");
        assert_eq!(session.description, "Grow from the initial 3.");
        assert_eq!(session.star_descriptions[2], "Reach 20 population");
        assert_eq!(session.champion_name, "Basic bibite");
        assert!(app.challenge_is_running() && !app.challenge_end_shown);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_challenge_without_a_champion_or_a_scenario_that_is_not_one_does_not_start() {
        let Some((scenario, champion, dir)) = fixture("refuse") else {
            return;
        };
        let refused = LoadedWorld::load_challenge_start(&start(&scenario, None, true), 9, 1);
        assert!(
            refused.is_err(),
            "the button is not interactable without a champion"
        );
        // Started as a scenario: the champion's row is there, nothing runs.
        let (loaded, setup) =
            LoadedWorld::load_challenge_start(&start(&scenario, Some(&champion), false), 9, 1)
                .expect("starts as a scenario");
        assert!(setup.is_none());
        assert!(
            loaded
                .world
                .bibite_spawns
                .iter()
                .any(|row| row.tag == "Champion")
        );
        let plain = repo_file("testdata/real/Default.zip");
        assert!(
            LoadedWorld::load_challenge_start(&start(&plain, Some(&champion), true), 9, 1).is_err()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_run_ends_pauses_and_scores_then_continues_as_a_scenario() {
        let Some((scenario, champion, dir)) = fixture("run") else {
            return;
        };
        let (loaded, setup) =
            LoadedWorld::load_challenge_start(&start(&scenario, Some(&champion), true), 10, 1)
                .expect("loads");
        let mut app = App::new(loaded, Vec2::new(1280.0, 800.0), None);
        app.storage = bibites_io::storage::StoragePaths::new(dir.join("state"));
        app.begin_challenge(setup.expect("setup"));
        app.paused = false;
        // 30 s of simulated time: the exit condition (`Time >= 30`) is met at a once-a-second check.
        let fixed_dt = app.world.settings.fixed_dt();
        for _ in 0..(40 * 40) {
            app.world.tick();
            app.update_challenge(fixed_dt);
            if app.challenge_end_shown {
                break;
            }
        }
        assert!(
            app.challenge_end_shown,
            "the exit condition ended the challenge"
        );
        assert!(app.paused, "the end pauses the game (`ChallengeEnd`)");
        assert_eq!(app.challenge_end_pause, Some(false));
        assert!(
            app.challenge_record.is_some(),
            "the result was stored (`TrySetHighScoreOfChallenge`)"
        );
        // `ContinueAsScenario`: the restrictions go, the end pause is given back.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::ChallengeContinue)]);
        assert!(app.challenge.is_none() && !app.challenge_is_running() && !app.challenge_end_shown);
        assert!(!app.paused);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Everything the extra panels of the HUD (tracker, tab, pause menu, end popup) offer as actions.
    fn extra_actions(app: &App) -> Vec<UiAction> {
        let mut frame = crate::ui::UiFrame::new(1.0);
        crate::hud::draw_extra_panels(
            &mut frame,
            app,
            &crate::ui::UiAssets::empty(),
            None,
            1920.0,
            1080.0,
            false,
        );
        frame.actions.iter().map(|(_, action)| *action).collect()
    }

    #[test]
    fn the_tab_opens_the_tracker_which_closes_again_and_retry_asks_first() {
        use crate::ui::PanelKind;
        let Some((scenario, champion, dir)) = fixture("tracker") else {
            return;
        };
        let (loaded, setup) =
            LoadedWorld::load_challenge_start(&start(&scenario, Some(&champion), true), 12, 1)
                .expect("loads");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        // Not a challenge yet: no tab. Beginning one shows "Challenge +" under the Information tab, the tracker closed.
        assert!(!extra_actions(&app).contains(&UiAction::OpenPanel(PanelKind::ChallengeTracker)));
        app.begin_challenge(setup.expect("setup"));
        assert!(extra_actions(&app).contains(&UiAction::OpenPanel(PanelKind::ChallengeTracker)));
        // The tab opens the panel (`ChallengeTrackerPanel.OpenPanel`); the tabs go (`onOpen`), the panel has its buttons.
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::ChallengeTracker)]);
        assert!(app.panels.is_open(PanelKind::ChallengeTracker));
        let open = extra_actions(&app);
        assert!(!open.contains(&UiAction::OpenPanel(PanelKind::ChallengeTracker)));
        assert!(open.contains(&UiAction::Invoke(ControlCommand::ChallengeTrackerClose)));
        assert!(open.contains(&UiAction::Invoke(ControlCommand::ChallengeRestart)));
        // The retry button asks (`DisplayWarningWithCancel`): nothing is thrown away until YES.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::ChallengeRestart)]);
        assert!(app.confirm_dialog.is_some() && app.challenge.is_some());
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Cancel)]);
        // Escape closes the panel (an escapable `UIPanel`), the close button does too.
        app.handle_input(&crate::app::FrameInput {
            quit: true,
            ..crate::app::FrameInput::default()
        });
        assert!(!app.panels.is_open(PanelKind::ChallengeTracker));
        assert!(
            !app.panels.is_open(PanelKind::Escape),
            "Escape closed the panel only"
        );
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::ChallengeTracker)]);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::ChallengeTrackerClose)]);
        assert!(!app.panels.is_open(PanelKind::ChallengeTracker));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_challenge_takes_save_and_load_out_of_the_pause_menu() {
        let Some((scenario, champion, dir)) = fixture("pause") else {
            return;
        };
        let (loaded, setup) =
            LoadedWorld::load_challenge_start(&start(&scenario, Some(&champion), true), 13, 1)
                .expect("loads");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenEscapeMenu)]);
        let plain = extra_actions(&app);
        assert!(plain.contains(&UiAction::Invoke(ControlCommand::OpenSavePanel)));
        assert!(plain.contains(&UiAction::Invoke(ControlCommand::OpenLoadPanel)));
        // `ChallengeManager.objectsToHide`: "You cannot save a challenge".
        app.begin_challenge(setup.expect("setup"));
        let hidden = extra_actions(&app);
        assert!(!hidden.contains(&UiAction::Invoke(ControlCommand::OpenSavePanel)));
        assert!(!hidden.contains(&UiAction::Invoke(ControlCommand::OpenLoadPanel)));
        assert!(hidden.contains(&UiAction::Invoke(ControlCommand::OpenSettings)));
        assert!(hidden.contains(&UiAction::Invoke(ControlCommand::OpenLibrary)));
        // The two are all the menu loses (the Challenge tab is what the challenge adds).
        let lost: Vec<_> = plain
            .iter()
            .filter(|action| !hidden.contains(action))
            .collect();
        assert_eq!(lost.len(), 2, "{lost:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_end_popup_blocks_the_keyboard_and_its_buttons_end_the_challenge() {
        let Some((scenario, champion, dir)) = fixture("popup") else {
            return;
        };
        let (loaded, setup) =
            LoadedWorld::load_challenge_start(&start(&scenario, Some(&champion), true), 14, 1)
                .expect("loads");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        app.storage = bibites_io::storage::StoragePaths::new(dir.join("state"));
        app.begin_challenge(setup.expect("setup"));
        assert!(
            !extra_actions(&app).contains(&UiAction::Invoke(ControlCommand::ChallengeContinue)),
            "no popup while the challenge runs"
        );
        app.paused = false;
        let fixed_dt = app.world.settings.fixed_dt();
        for _ in 0..(40 * 40) {
            app.world.tick();
            app.update_challenge(fixed_dt);
            if app.challenge_end_shown {
                break;
            }
        }
        assert!(app.challenge_end_shown);
        let popup = extra_actions(&app);
        assert!(popup.contains(&UiAction::Invoke(ControlCommand::ChallengeContinue)));
        assert!(popup.contains(&UiAction::Invoke(ControlCommand::ChallengeReturnToMenu)));
        // `UserControl.SetKeyboardBlockFromSource("Challenge", true)`: neither Space nor Escape does anything.
        let paused = app.paused;
        app.handle_input(&crate::app::FrameInput {
            toggle_pause: true,
            ..crate::app::FrameInput::default()
        });
        assert_eq!(app.paused, paused);
        app.handle_input(&crate::app::FrameInput {
            quit: true,
            ..crate::app::FrameInput::default()
        });
        assert!(!app.panels.is_open(crate::ui::PanelKind::Escape));
        // "Continue as a Simulation" goes on as a scenario: the popup is gone, the controls are back.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::ChallengeContinue)]);
        assert!(
            !extra_actions(&app).contains(&UiAction::Invoke(ControlCommand::ChallengeContinue))
        );
        assert!(!app.challenge_is_running());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn retry_builds_the_same_challenge_again_and_return_to_menu_leaves() {
        let Some((scenario, champion, dir)) = fixture("retry") else {
            return;
        };
        let (loaded, setup) =
            LoadedWorld::load_challenge_start(&start(&scenario, Some(&champion), true), 11, 1)
                .expect("loads");
        let mut app = App::new(loaded, Vec2::new(1280.0, 800.0), None);
        app.begin_challenge(setup.expect("setup"));
        for _ in 0..400 {
            app.world.tick();
        }
        let before = app.world.simulated_time();
        assert!(before > 5.0);
        // "Retry Challenge" asks first (`DisplayWarningWithCancel`), YES starts it again.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::ChallengeRestart)]);
        assert!(app.confirm_dialog.is_some());
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Confirm)]);
        assert!(app.world.simulated_time() < before, "a new world");
        let session = app.challenge.as_ref().expect("a fresh session");
        assert_eq!(session.challenge_name, "Little Rabbits");
        assert!(session.elapsed_s < 1.0 && !app.challenge_end_shown);
        // The popup's other button.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::ChallengeReturnToMenu)]);
        assert!(app.return_to_menu && app.challenge.is_none() && app.challenge_setup.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
