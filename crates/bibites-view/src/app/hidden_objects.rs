//! RUSTibites Improved: "Hide Objects", a hotkey (V until the player changes it on the Extra Hotkeys page of the Settings screens) that hides the world's objects (bibites,
//! eggs, pellets, towers, the particles) and leaves the interface: the panels, the graphs, the zones and the grid. It is a way of looking, not a setting: it is not stored,
//! a window starts with the objects shown, and the simulation does not notice it (the scene is built without them, `SceneOptions::objects`).

use super::App;

impl App {
    /// The Hide Objects hotkey: the objects are hidden, or shown again.
    pub fn toggle_objects(&mut self) {
        self.scene_options.objects = !self.scene_options.objects;
        self.last_action = if self.scene_options.objects { "objects shown" } else { "objects hidden" }.to_owned();
    }

    /// Whether the world's objects are drawn now (the labels and bars that hang on a bibite follow them).
    #[must_use]
    pub fn objects_shown(&self) -> bool {
        self.scene_options.objects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{FrameInput, LoadedWorld};
    use bibites_core::math::Vec2;

    fn app() -> App {
        let loaded = LoadedWorld::load(None, 77, 6, 1).expect("starter world");
        App::new(loaded, Vec2::new(1920.0, 1080.0), None)
    }

    fn pressed() -> FrameInput {
        FrameInput { toggle_objects: true, dt: 1.0 / 60.0, ..FrameInput::default() }
    }

    #[test]
    fn the_hotkey_hides_and_shows_the_objects_and_a_window_starts_with_them_shown() {
        let mut app = app();
        assert!(app.objects_shown(), "shown from the start");
        app.handle_input(&pressed());
        assert!(!app.objects_shown());
        assert_eq!(app.last_action, "objects hidden");
        app.handle_input(&pressed());
        assert!(app.objects_shown());
        assert_eq!(app.last_action, "objects shown");
    }

    #[test]
    fn hiding_the_objects_does_not_touch_the_simulation_and_the_scene_has_none_of_them() {
        use crate::render::Primitive;
        let mut shown = app();
        let mut hidden = app();
        hidden.handle_input(&pressed());
        for app in [&mut shown, &mut hidden] {
            // (Without the game's art a zone is a circle too: the zones are left out so that what is counted below is the objects.)
            app.scene_options.zones = false;
            for _ in 0..30 {
                app.tick_world();
            }
            app.rebuild_scene();
        }
        // The world is the same world, tick for tick.
        assert_eq!(shown.world.bibites.len(), hidden.world.bibites.len());
        assert_eq!(shown.world.simulated_time().to_bits(), hidden.world.simulated_time().to_bits());
        assert!(!shown.world.bibites.is_empty(), "the starter world has bibites");
        // The picture: the same background and zones, without the objects (they are bodies, sprites and dots).
        let objects = |app: &App| {
            app.scene
                .primitives
                .iter()
                .filter(|primitive| matches!(primitive, Primitive::Capsule { .. } | Primitive::Sprite { .. } | Primitive::Dots { count: 1.., .. } | Primitive::Circle { .. }))
                .count()
        };
        assert!(objects(&shown) > 0, "something to hide");
        assert!(hidden.scene.len() < shown.scene.len());
        assert_eq!(objects(&hidden), 0, "no bibite, egg or pellet is drawn");
        assert!(hidden.scene.len() > 0, "the background is");
    }

    #[test]
    fn the_hotkey_waits_while_something_else_has_the_keyboard() {
        let mut app = app();
        // A tag being typed: the key is a letter of it.
        app.tag_edit = Some(String::new());
        app.handle_input(&pressed());
        assert!(app.objects_shown(), "typing is not a hotkey");
        app.tag_edit = None;
        app.handle_input(&pressed());
        assert!(!app.objects_shown());
    }
}
