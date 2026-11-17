//! The achievements (`SteamIntegrations.AchievementManager`, `Achievement`, `ChallengeStarsAchievement`, `UserActionCount`,
//! `AchievementPopupHandle`): which ones the player has, how a trigger unlocks one, and the popup that tells.
//!
//! The game keeps an unlock as `PlayerPrefs.SetInt("ACH_...", 1)` (and on Steam); this port reads the installed game's flags
//! (`GamePrefs`, read-only) and keeps its own unlocks and counters in `achievements.json` of the state folder, so a result is
//! "unlocked" when either says so. `Achievement.Trigger` does nothing for one that is already unlocked, else it unlocks it and
//! `TooltipSystem.TriggerAchievementPopup` shows the popup (`TopLayer/TooltipCanvas/AchievementPopup`: bottom right, 13 from both edges).
//!
//! The popup (prefab values, level2 `AchievementPopup`): a `base_icon` (ppum 0.5) panel with a horizontal layout padded 15 / 20 / 15 / 15
//! and 20 apart: the icon in a 108 x 108 `base_icon` border (96 x 96, 6 inside), then the text column: the title (Roboto-Bold 24, black)
//! over the description (Roboto-Regular 24, (42, 42, 42)) 2 below it. The description's layout element asks for a preferred width of 0,
//! so the column is as wide as the title and the description wraps to it. It stays 5 s, then fades by half an alpha a second, unless the
//! pointer is on it (`WaitAndFadeOut`); a click on it selects the bibite that unlocked it, which no trigger passes on (the popup's source
//! is always null), so it does nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use bibites_core::math::Vec2;
use bibites_io::prefs::GamePrefs;
use bibites_io::storage::{AchievementsFile, StoragePaths};

use crate::species_panel::Drawer;
use crate::ui::{FontRole, Rgba, UiAction, UiDraw, UiFrame};

/// `AchievementManager.achievements`: one achievement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Def {
    /// `ACH_...`.
    pub id: String,
    /// The title.
    pub title: String,
    /// The description.
    pub desc: &'static str,
    /// `secret`.
    pub secret: bool,
}

/// The four challenges and their three stars' descriptions (`ChallengeStarsAchievement`).
const CHALLENGES: [(&str, [&str; 3]); 4] = [
    (
        "Apocalypse",
        [
            "Your champion survived a decaying world for an hour",
            "Your champion survived a decaying world for 3 hours",
            "Your champion survived a decaying world for 10 hours",
        ],
    ),
    (
        "Like Rabbits",
        [
            "Your champion managed to reach up to 50 population in an hour!",
            "Your champion managed to reach up to 100 population in an hour!",
            "Your champion managed to reach up to 200 population in an hour!",
        ],
    ),
    (
        "Down with the Basics",
        [
            "Your champion eradicated the basic bibites in less than 2 hours",
            "Your champion eradicated the basic bibites in less than 1 hour",
            "Your champion eradicated the basic bibites in less than 30 minutes",
        ],
    ),
    (
        "The Ultimate Duel",
        [
            "Your champion eradicated the Darth bibitus in less than 2 hours",
            "Your champion eradicated the Darth bibitus in less than 1 hour",
            "Your champion eradicated the Darth bibitus in less than 30 minutes",
        ],
    ),
];

/// `ChallengesProgress.NameToAchievementKey`: `"ACH_" + name without spaces, upper case + "_{level}STAR"`.
#[must_use]
pub fn challenge_id(challenge: &str, level: u8) -> String {
    format!(
        "ACH_{}_{level}STAR",
        challenge.replace(' ', "").to_ascii_uppercase()
    )
}

/// Every achievement in the game's order: the eight of the simulation, then three stars of each challenge.
#[must_use]
pub fn defs() -> Vec<Def> {
    let base = |id: &str, title: &str, desc: &'static str, secret: bool| Def {
        id: id.to_owned(),
        title: title.to_owned(),
        desc,
        secret,
    };
    let mut list = vec![
        base(
            "ACH_BIBITE_EDITOR",
            "Bibite Engineer",
            "Save your first engineered bibite!",
            false,
        ),
        base(
            "ACH_BIBITE_EDITOR_100",
            "Master Manipulator",
            "Save your hundredth engineered bibite",
            false,
        ),
        base(
            "ACH_BIBITE_EINSTEIN",
            "Einstein",
            "Have a bibite born with a brain containing 100 hidden nodes",
            false,
        ),
        base(
            "ACH_SIM_7DAYS",
            "Let There Be Light",
            "Reach 7 days (168h) of simulation time",
            false,
        ),
        base(
            "ACH_SIM_1FPS",
            "Does anyone smell bacon?",
            "Stay below 1FPS for a minute",
            true,
        ),
        base(
            "ACH_BIBITE_1DAY",
            "The Elder among us",
            "Have a bibite older than a full day",
            false,
        ),
        base(
            "ACH_BIBITE_100CHILDREN",
            "The Matriarch",
            "Have a bibite with 100 alive children",
            false,
        ),
        base(
            "ACH_USER_SNAP",
            "I am Become Death",
            "Kill half the population of your sim in a single click...\nYou monster...",
            false,
        ),
    ];
    for (name, descriptions) in CHALLENGES {
        for (index, desc) in descriptions.into_iter().enumerate() {
            let level = index as u8 + 1;
            list.push(Def {
                id: challenge_id(name, level),
                title: format!(
                    "{name} Challenge - {level} Star{}",
                    if level > 1 { "s" } else { "" }
                ),
                desc,
                secret: false,
            });
        }
    }
    list
}

/// The achievement with an id.
#[must_use]
pub fn def_of(id: &str) -> Option<Def> {
    defs().into_iter().find(|def| def.id == id)
}

/// The unlocks the screens that have no [`Achievements`] at hand (the title menu's gallery) look at: a copy of the unlocked set.
static SHARED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Whether this port unlocked `id` (the installed game's own flags are `GamePrefs`').
#[must_use]
pub fn shared_unlocked(id: &str) -> bool {
    SHARED.lock().is_ok_and(|set| set.contains(id))
}

/// Loads the stored unlocks into the shared copy (the title menu is drawn before any world exists).
pub fn publish_stored(paths: &StoragePaths) {
    if let Ok(file) = bibites_io::storage::read_achievements(paths)
        && let Ok(mut shared) = SHARED.lock()
    {
        shared.extend(file.unlocked);
    }
}

/// The popup that shows an unlock.
#[derive(Clone, Debug, PartialEq)]
pub struct Popup {
    /// The achievement.
    pub id: String,
    /// Seconds since it came up (real time).
    pub age: f32,
    /// `CanvasGroup.alpha`.
    pub alpha: f32,
}

/// `AchievementPopupHandle.initialWait`.
const SHOW_SECONDS: f32 = 5.0;
/// How fast the popup fades once it may (`alpha - dt / 2`).
const FADE_PER_SECOND: f32 = 0.5;

/// The player's achievements.
#[derive(Clone, Debug, Default)]
pub struct Achievements {
    unlocked: BTreeSet<String>,
    counters: BTreeMap<String, u64>,
    paths: Option<StoragePaths>,
    /// The installed game's own flags and counters (read-only; empty without the game and in the tests).
    game: GamePrefs,
    /// The popup, while there is one.
    pub popup: Option<Popup>,
    /// `TimeKeeper.triggered7Days`.
    pub triggered_seven_days: bool,
    /// `UserActions.triggeredThanos`.
    pub triggered_snap: bool,
    /// `TimeKeeper.timeBellow1FPS`.
    pub seconds_below_one_fps: f32,
    /// `TimeKeeper.framesBellow1FPS`.
    pub frames_below_one_fps: u32,
    /// Where the popup was drawn last (reference px), for the pointer's hover the next frame.
    pub shown_at: std::cell::Cell<Option<[f32; 4]>>,
    /// Seconds since the world was scanned for the achievements that need a look at it.
    pub scan_timer: f32,
}

impl Achievements {
    /// The stored unlocks and counters of this port (nothing when the file is missing or unreadable); later unlocks are written back to it.
    /// The installed game's flags count as unlocks too.
    #[must_use]
    pub fn load(paths: &StoragePaths) -> Self {
        Self::load_with(paths, GamePrefs::global().clone())
    }

    /// [`Self::load`] with the installed game's preferences given.
    #[must_use]
    pub fn load_with(paths: &StoragePaths, game: GamePrefs) -> Self {
        let file = bibites_io::storage::read_achievements(paths).unwrap_or_default();
        let achievements = Self {
            unlocked: file.unlocked,
            counters: file.counters,
            paths: Some(paths.clone()),
            game,
            ..Self::default()
        };
        achievements.publish();
        achievements
    }

    /// Achievements that know the installed game's preferences but keep nothing of their own in a file (the tests').
    #[cfg(test)]
    #[must_use]
    pub fn with_game(game: GamePrefs) -> Self {
        Self {
            game,
            ..Self::default()
        }
    }

    fn publish(&self) {
        if let Ok(mut shared) = SHARED.lock() {
            shared.extend(self.unlocked.iter().cloned());
        }
    }

    fn store(&self) {
        let Some(paths) = &self.paths else {
            return;
        };
        let file = AchievementsFile {
            unlocked: self.unlocked.clone(),
            counters: self.counters.clone(),
        };
        let _ = bibites_io::storage::write_achievements(paths, &file);
    }

    /// Whether `id` is unlocked: by this port, or by the installed game (`Achievement.achievedLocal`).
    #[must_use]
    pub fn is_unlocked(&self, id: &str) -> bool {
        self.unlocked.contains(id) || self.game.flag(id) == Some(true)
    }

    /// `AchievementManager.Trigger(id)`: unlocks `id` (stored, the popup comes up) unless it is unlocked already. Returns whether it was
    /// unlocked now.
    pub fn trigger(&mut self, id: &str) -> bool {
        if def_of(id).is_none() || self.is_unlocked(id) {
            return false;
        }
        self.unlocked.insert(id.to_owned());
        self.publish();
        self.store();
        // `AchievementPopupHandle.UnlockAchievement`: a new unlock takes the popup over from the one that was showing.
        self.popup = Some(Popup {
            id: id.to_owned(),
            age: 0.0,
            alpha: 1.0,
        });
        true
    }

    /// `UserActionCount.Trigger`: the counter goes up by one and the game returns the count after it plus one (`SetValue(val + 1)` then
    /// `return val + 1`), which is what `> 100` is tested against. A counter this port has not kept starts from the game's own value.
    pub fn count(&mut self, name: &str) -> u64 {
        let start = self.game.int(name).unwrap_or(0).max(0) as u64;
        let value = self.counters.entry(name.to_owned()).or_insert(start);
        *value += 1;
        let after = *value;
        self.store();
        after + 1
    }

    /// One frame of the popup's life (`WaitAndFadeOut`): `dt` real seconds, `hovered` whether the pointer is on it.
    pub fn tick(&mut self, dt: f32, hovered: bool) {
        let Some(popup) = self.popup.as_mut() else {
            return;
        };
        let before = popup.age;
        popup.age += dt;
        if popup.age < SHOW_SECONDS {
            popup.alpha = 1.0;
            return;
        }
        // Only the part of the frame past the wait fades it, and the pointer on it holds it.
        if !hovered {
            popup.alpha -= (popup.age - SHOW_SECONDS.max(before)) * FADE_PER_SECOND;
        }
        popup.alpha = popup.alpha.clamp(0.0, 1.0);
        if popup.alpha <= 0.0 {
            self.popup = None;
        }
    }
}

/// The sprite of an achievement's icon: its own `ACH_..._1` (unlocked) or `_0` set when the game has one, else the placeholders.
#[must_use]
pub fn icon_names(id: &str, achieved: bool) -> [String; 2] {
    [
        format!("{id}_{}", u8::from(achieved)),
        if achieved {
            "rockstar_placeholder_colored".to_owned()
        } else {
            "rockstar_placeholder_grayscale".to_owned()
        },
    ]
}

// ---------------------------------------------------------------------------------------- the popup's picture

/// Prefab geometry (reference px).
const MARGIN: f32 = 13.0;
const PADDING: [f32; 4] = [15.0, 20.0, 15.0, 15.0];
const SPACING: f32 = 20.0;
const ICON_BORDER: f32 = 108.0;
const ICON: f32 = 96.0;
const TEXT_SIZE: f32 = 24.0;
/// The line height of a 24-point Roboto line.
const LINE: f32 = TEXT_SIZE * 1.17;
const HIT_POPUP: u32 = 8_150;

/// The popup's laid-out rectangle and text.
#[derive(Clone, Debug, PartialEq)]
pub struct PopupLayout {
    /// The panel.
    pub rect: [f32; 4],
    /// The icon's border box.
    pub icon_border: [f32; 4],
    /// The icon.
    pub icon: [f32; 4],
    /// The text column's left edge and the title's top.
    pub text_at: [f32; 2],
    /// The description's lines.
    pub desc_lines: Vec<String>,
}

/// Lays the popup of `def` out for a window of `window` reference px: as wide as the title asks (the description wraps to it), as high as the
/// taller of the icon and the text.
#[must_use]
pub fn layout(def: &Def, window: [f32; 2], measure: &dyn Fn(&str, bool) -> f32) -> PopupLayout {
    let title_width = measure(&def.title, true);
    let desc_lines = crate::tooltip::wrap(def.desc, title_width, &|text| measure(text, false));
    let text_width = title_width;
    let text_height = LINE + 2.0 + LINE * desc_lines.len() as f32;
    let width = PADDING[0] + ICON_BORDER + SPACING + text_width + PADDING[1];
    let height = PADDING[2] + ICON_BORDER.max(text_height) + PADDING[3];
    let rect = [
        window[0] - MARGIN - width,
        window[1] - MARGIN - height,
        width,
        height,
    ];
    let icon_border = [rect[0] + PADDING[0], rect[1] + PADDING[2], ICON_BORDER, ICON_BORDER];
    let icon = [
        icon_border[0] + (ICON_BORDER - ICON) / 2.0,
        icon_border[1] + (ICON_BORDER - ICON) / 2.0,
        ICON,
        ICON,
    ];
    PopupLayout {
        rect,
        icon_border,
        icon,
        text_at: [icon_border[0] + ICON_BORDER + SPACING, rect[1] + PADDING[2]],
        desc_lines,
    }
}

fn faded(colour: Rgba, alpha: f32) -> Rgba {
    [
        colour[0],
        colour[1],
        colour[2],
        (f32::from(colour[3]) * alpha).round() as u8,
    ]
}

/// Draws the popup over the window (`width` x `height` px), at its alpha, and takes the clicks on it like the game's raycast target.
pub fn draw(frame: &mut UiFrame, d: &Drawer<'_>, state: &Achievements, width: f32, height: f32) {
    let Some(popup) = state.popup.as_ref() else {
        state.shown_at.set(None);
        return;
    };
    let Some(def) = def_of(&popup.id) else {
        return;
    };
    let s = d.s;
    let measure = |text: &str, bold: bool| d.measure(text, TEXT_SIZE * s, bold) / s;
    let place = layout(&def, [width / s, height / s], &measure);
    let alpha = popup.alpha;
    state.shown_at.set(Some(place.rect));
    d.slice(frame, "base_icon", 5.0, 2.0, place.rect, faded([255, 255, 255, 255], alpha));
    d.slice(
        frame,
        "base_icon",
        5.0,
        2.0,
        place.icon_border,
        faded([255, 255, 255, 255], alpha),
    );
    // The icon is the unlocked one (`spriteAchieved`).
    for name in icon_names(&def.id, true) {
        if d.ui.texture_of_alias(&name).is_some() {
            d.icon(frame, &name, place.icon, faded([255, 255, 255, 255], alpha));
            break;
        }
    }
    frame.draws.push(UiDraw::Text {
        font: Some("Roboto-Bold SDF NoOutline".to_owned()),
        text: def.title.clone(),
        pos: Vec2::new(place.text_at[0] * s, place.text_at[1] * s),
        px: TEXT_SIZE * s,
        color: faded([0, 0, 0, 255], alpha),
        role: FontRole::Bold,
    });
    for (index, line) in place.desc_lines.iter().enumerate() {
        frame.draws.push(UiDraw::Text {
            font: Some("Roboto-Regular SDF".to_owned()),
            text: line.clone(),
            pos: Vec2::new(
                place.text_at[0] * s,
                (place.text_at[1] + LINE + 2.0 + LINE * index as f32) * s,
            ),
            px: TEXT_SIZE * s,
            color: faded([42, 42, 42, 255], alpha),
            role: FontRole::Body,
        });
    }
    // The panel is a raycast target: a press on it does not reach what is under it.
    frame.hit(
        HIT_POPUP,
        d.at(place.rect[0], place.rect[1]),
        d.at(place.rect[2], place.rect[3]),
        UiAction::Consume,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> StoragePaths {
        let dir = std::env::temp_dir().join(format!("rustibites_ach_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        StoragePaths::new(dir)
    }

    #[test]
    fn the_list_is_the_games_eight_and_twelve_stars_with_their_keys() {
        let list = defs();
        assert_eq!(list.len(), 20);
        assert_eq!(list[0].title, "Bibite Engineer");
        assert!(list[4].secret && list[4].id == "ACH_SIM_1FPS");
        assert_eq!(list[7].desc, "Kill half the population of your sim in a single click...\nYou monster...");
        // `NameToAchievementKey`: the name without spaces, upper case, and the level.
        assert_eq!(challenge_id("Like Rabbits", 2), "ACH_LIKERABBITS_2STAR");
        assert_eq!(list[8].id, "ACH_APOCALYPSE_1STAR");
        assert_eq!(list[8].title, "Apocalypse Challenge - 1 Star");
        assert_eq!(list[19].id, "ACH_THEULTIMATEDUEL_3STAR");
        assert_eq!(list[19].title, "The Ultimate Duel Challenge - 3 Stars");
        // The gallery's ids are all there.
        for id in [
            "ACH_BIBITE_EDITOR",
            "ACH_BIBITE_EDITOR_100",
            "ACH_BIBITE_EINSTEIN",
            "ACH_SIM_7DAYS",
            "ACH_BIBITE_1DAY",
            "ACH_BIBITE_100CHILDREN",
            "ACH_USER_SNAP",
        ] {
            assert!(def_of(id).is_some(), "{id}");
        }
        assert!(def_of("ACH_NOPE").is_none());
    }

    #[test]
    fn a_trigger_unlocks_once_stores_it_and_shows_the_popup() {
        let paths = temp_paths("trigger");
        let mut state = Achievements::load_with(&paths, GamePrefs::default());
        assert!(!state.is_unlocked("ACH_SIM_7DAYS"));
        assert!(state.trigger("ACH_SIM_7DAYS"));
        assert!(state.is_unlocked("ACH_SIM_7DAYS"));
        assert_eq!(state.popup.as_ref().expect("popup").id, "ACH_SIM_7DAYS");
        // Again: nothing (`Achievement.Trigger` checks `achieved`), and an unknown id is not an achievement.
        state.popup = None;
        assert!(!state.trigger("ACH_SIM_7DAYS"));
        assert!(state.popup.is_none());
        assert!(!state.trigger("ACH_NOPE"));
        // It is stored and the shared copy has it.
        let file = bibites_io::storage::read_achievements(&paths).expect("file");
        assert!(file.unlocked.contains("ACH_SIM_7DAYS"));
        assert!(shared_unlocked("ACH_SIM_7DAYS"));
        let again = Achievements::load_with(&paths, GamePrefs::default());
        assert!(again.is_unlocked("ACH_SIM_7DAYS"));
        // What the installed game has unlocked counts too.
        let game = GamePrefs::parse("i\tACH_USER_SNAP\t1\n");
        let mut other = Achievements::with_game(game);
        assert!(other.is_unlocked("ACH_USER_SNAP"));
        assert!(!other.trigger("ACH_USER_SNAP"), "already unlocked in the game");
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn a_new_unlock_takes_the_popup_over_and_the_popup_waits_five_seconds_then_fades_unless_hovered() {
        let mut state = Achievements::default();
        state.trigger("ACH_SIM_7DAYS");
        state.tick(4.9, false);
        assert_eq!(state.popup.as_ref().expect("popup").alpha, 1.0);
        // Past the five seconds it fades by half an alpha a second.
        state.tick(0.1, false);
        state.tick(1.0, false);
        let alpha = state.popup.as_ref().expect("popup").alpha;
        assert!((alpha - 0.5).abs() < 0.01, "{alpha}");
        // The pointer on it holds the alpha where it is.
        state.tick(5.0, true);
        assert!((state.popup.as_ref().expect("popup").alpha - 0.5).abs() < 0.01);
        // A new unlock replaces it, at full alpha again.
        state.trigger("ACH_USER_SNAP");
        let popup = state.popup.as_ref().expect("popup");
        assert_eq!((popup.id.as_str(), popup.alpha, popup.age), ("ACH_USER_SNAP", 1.0, 0.0));
        // Unhovered, it is gone after another two seconds past the five.
        state.tick(5.0, false);
        state.tick(1.0, false);
        state.tick(1.1, false);
        assert!(state.popup.is_none());
    }

    #[test]
    fn the_counter_goes_up_by_one_and_returns_the_game_s_one_more_than_that() {
        let paths = temp_paths("counter");
        let mut state = Achievements::load_with(&paths, GamePrefs::default());
        // `UserActionCount.Trigger` returns the count after the save plus one: the hundredth save is the first over 100.
        for save in 1..=99_u64 {
            assert_eq!(state.count("BibiteEditorSaved"), save + 1);
        }
        assert_eq!(state.count("BibiteEditorSaved"), 101);
        // It is stored, and a counter this port has not kept starts from the game's.
        let mut again = Achievements::load_with(&paths, GamePrefs::default());
        assert_eq!(again.count("BibiteEditorSaved"), 102);
        let mut fresh = Achievements::with_game(GamePrefs::parse("i\tBibiteEditorSaved\t98\n"));
        assert_eq!(fresh.count("BibiteEditorSaved"), 100);
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn the_popup_is_as_wide_as_its_title_and_wraps_the_description_to_it() {
        // 12 px a character, bold or not.
        let measure = |text: &str, _bold: bool| text.chars().count() as f32 * 12.0;
        let def = def_of("ACH_BIBITE_EDITOR").expect("editor");
        let place = layout(&def, [1920.0, 1080.0], &measure);
        let title = "Bibite Engineer".chars().count() as f32 * 12.0;
        let width = 15.0 + 108.0 + 20.0 + title + 20.0;
        assert!((place.rect[2] - width).abs() < 1e-3);
        // 13 from the bottom right corner.
        assert!((place.rect[0] + place.rect[2] - (1920.0 - 13.0)).abs() < 1e-3);
        assert!((place.rect[1] + place.rect[3] - (1080.0 - 13.0)).abs() < 1e-3);
        assert!(place.desc_lines.len() >= 2, "{:?}", place.desc_lines);
        for line in &place.desc_lines {
            assert!(
                line.chars().count() as f32 * 12.0 <= title + 1e-3
                    || !line.contains(' '),
                "{line:?}"
            );
        }
        // The icon sits 6 inside its 108-wide border.
        assert_eq!(place.icon[0] - place.icon_border[0], 6.0);
        // Height: the taller of the icon's border and the text, plus 15 + 15.
        let text_height = 24.0 * 1.17 * (1.0 + place.desc_lines.len() as f32) + 2.0;
        assert!((place.rect[3] - (30.0 + 108.0_f32.max(text_height))).abs() < 1e-3);
        // A long title makes it wide, a long description tall.
        let long = def_of("ACH_THEULTIMATEDUEL_3STAR").expect("long");
        assert!(layout(&long, [1920.0, 1080.0], &measure).rect[2] > place.rect[2]);
    }

    #[test]
    fn the_icons_are_the_unlocked_set_or_the_placeholders() {
        assert_eq!(
            icon_names("ACH_BIBITE_EDITOR", true),
            [
                "ACH_BIBITE_EDITOR_1".to_owned(),
                "rockstar_placeholder_colored".to_owned()
            ]
        );
        assert_eq!(icon_names("ACH_SIM_1FPS", false)[1], "rockstar_placeholder_grayscale");
    }

    // ------------------------------------------------------------------------------------ the triggers, through the app

    use crate::app::{App, LoadedWorld};

    fn app() -> App {
        let loaded = LoadedWorld::load(None, 31, 8, 1).expect("starter world");
        App::new(loaded, Vec2::new(1920.0, 1080.0), None)
    }

    #[test]
    fn killing_half_the_population_unlocks_the_snap_once_and_shows_its_popup() {
        let mut app = app();
        assert!(!app.achievements.is_unlocked("ACH_USER_SNAP"));
        let kill = |app: &mut App| {
            app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::RequestKillHalf)]);
            app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::Confirm)]);
        };
        kill(&mut app);
        assert!(app.achievements.is_unlocked("ACH_USER_SNAP"));
        assert_eq!(
            app.achievements.popup.as_ref().map(|popup| popup.id.as_str()),
            Some("ACH_USER_SNAP")
        );
        // A second time nothing changes (`triggeredThanos`, and it is unlocked).
        app.achievements.popup = None;
        kill(&mut app);
        assert!(app.achievements.popup.is_none());
    }

    #[test]
    fn a_hundred_and_sixty_eight_hours_of_simulated_time_unlock_the_light() {
        let mut app = app();
        app.world.set_simulated_time(167.0 * 3600.0);
        app.advance(0.016);
        assert!(!app.achievements.is_unlocked("ACH_SIM_7DAYS"));
        app.world.set_simulated_time(168.5 * 3600.0);
        app.advance(0.016);
        assert!(app.achievements.is_unlocked("ACH_SIM_7DAYS"));
        assert!(app.achievements.triggered_seven_days);
    }

    #[test]
    fn a_bibite_older_than_a_day_and_one_with_a_hundred_children_are_found_by_the_scan() {
        let mut app = app();
        // The scan looks once a second of real time.
        let id = app.world.live_bibite_ids()[0];
        app.world.bibites.get_mut(id).expect("bibite").clock.time_alive = 86_500.0;
        app.advance(0.5);
        assert!(!app.achievements.is_unlocked("ACH_BIBITE_1DAY"), "not looked at yet");
        app.advance(0.6);
        assert!(app.achievements.is_unlocked("ACH_BIBITE_1DAY"));
        // 100 children (any ids: the list's length counts).
        let record = app.world.bibites.get_mut(id).expect("bibite");
        record.children = vec![id; 100];
        app.advance(1.1);
        assert!(app.achievements.is_unlocked("ACH_BIBITE_100CHILDREN"));
    }

    #[test]
    fn a_minute_below_one_and_a_half_frames_a_second_unlocks_the_bacon() {
        let mut app = app();
        // Unpaused, with a measured frame rate under 1.5 for a minute of real time and more than ten frames.
        app.paused = false;
        let mut now = 0.0_f64;
        for _ in 0..70 {
            now += 1.0;
            app.time_keeper.render_frame(now, false);
            app.achievements_frame(1.0, true);
        }
        assert!(app.achievements.is_unlocked("ACH_SIM_1FPS"), "{:?}", app.time_keeper.fps());
    }

    #[test]
    fn the_editors_saves_unlock_the_engineer_and_the_hundredth_the_master() {
        let mut app = app();
        app.editor_saved_achievements();
        assert!(app.achievements.is_unlocked("ACH_BIBITE_EDITOR"));
        assert!(!app.achievements.is_unlocked("ACH_BIBITE_EDITOR_100"));
        for _ in 0..98 {
            app.editor_saved_achievements();
        }
        assert!(!app.achievements.is_unlocked("ACH_BIBITE_EDITOR_100"), "99 saves");
        app.editor_saved_achievements();
        assert!(app.achievements.is_unlocked("ACH_BIBITE_EDITOR_100"), "the hundredth");
    }

    #[test]
    fn a_challenges_stars_unlock_one_achievement_each() {
        let mut app = app();
        app.challenge_stars_achievements("Like Rabbits", 2);
        assert!(app.achievements.is_unlocked("ACH_LIKERABBITS_1STAR"));
        assert!(app.achievements.is_unlocked("ACH_LIKERABBITS_2STAR"));
        assert!(!app.achievements.is_unlocked("ACH_LIKERABBITS_3STAR"));
        assert!(!app.achievements.is_unlocked("ACH_APOCALYPSE_1STAR"));
    }

    #[test]
    fn the_popup_is_drawn_bottom_right_fades_with_time_and_takes_the_clicks_on_it() {
        let mut app = app();
        app.trigger_achievement("ACH_BIBITE_EINSTEIN");
        let assets = crate::ui::UiAssets::empty();
        let mut frame = UiFrame::new(1.0);
        let drawer = Drawer::new(1.0, &assets, None, app.cursor);
        draw(&mut frame, &drawer, &app.achievements, 1920.0, 1080.0);
        let rect = app.achievements.shown_at.get().expect("drawn");
        assert!((rect[0] + rect[2] - 1907.0).abs() < 0.5 && (rect[1] + rect[3] - 1067.0).abs() < 0.5);
        let texts: Vec<String> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"Einstein".to_owned()), "{texts:?}");
        // The panel takes the press (a hit with the consume action over its rectangle).
        let inside = Vec2::new(rect[0] + 20.0, rect[1] + 20.0);
        assert_eq!(frame.action_at(inside), Some(UiAction::Consume));
        // The pointer on it holds it; off it, it fades once the five seconds are over.
        app.cursor = inside;
        app.advance(0.016);
        app.achievements.tick(6.0, true);
        assert_eq!(app.achievements.popup.as_ref().expect("popup").alpha, 1.0);
        app.cursor = Vec2::new(5.0, 5.0);
        app.achievements.tick(1.0, false);
        assert!(app.achievements.popup.as_ref().expect("popup").alpha < 1.0);
        // Nothing drawn without a popup.
        app.achievements.popup = None;
        let mut empty = UiFrame::new(1.0);
        draw(&mut empty, &drawer, &app.achievements, 1920.0, 1080.0);
        assert!(empty.draws.is_empty() && app.achievements.shown_at.get().is_none());
    }
}
