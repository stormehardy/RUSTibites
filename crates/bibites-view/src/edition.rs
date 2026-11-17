//! Which edition of RUSTibites this build is.
//!
//! There are two: **Faithful** (the `main` branch: the game, reproduced as exactly as the port can) and **Improved** (the `improved` branch: the same
//! program with additions). The product's name lives here and nowhere else in the code, so that merging `main` into `improved` never fights over a
//! string: this file (with the zip name in `tools/release/build_alpha.ps1` and the `--help` header in `main.rs`) is what differs between the two. The
//! branch's rules are in `docs/IMPROVED.md`.

/// The name shown on the title screen's card, in the window's title bar and in the viewer's own HUD title.
pub const PRODUCT_NAME: &str = "RUSTibites Improved";

/// The title of the window while a world is open: "<product> - <world>".
pub fn world_window_title(world: &str) -> String {
    format!("{PRODUCT_NAME} - {world}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_improved_edition_is_named_improved_and_titles_a_world_window_with_it() {
        assert_eq!(PRODUCT_NAME, "RUSTibites Improved");
        assert_eq!(world_window_title("Default"), "RUSTibites Improved - Default");
    }
}
