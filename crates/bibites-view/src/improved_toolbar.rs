//! RUSTibites Improved: one more row on the View panel of the bottom-left toolbar (the panel with the "Fertility", "Pheromones", "Ruler" ... check boxes). The
//! panel is replayed from the game's capture, so the row is made from the capture's own: the "Fertility" row copied one row up (the panel grows by one row at
//! its top, where nothing is), with the label of the new switch.

use crate::ui_runtime::{Color, ImageComponent, Node, RuntimeSession, SpriteRef};

/// The name of the row's node (the toolbar's hit and check-mark code look for it).
pub const ZONE_COLORS_ROW: &str = "ZoneColors";
/// The row's label (the capture's rows are one short line each: the game's own "Fertility", "Pheromones", "Status bars").
pub const ZONE_COLORS_LABEL: &str = "Individual Zone Colors";
/// The distance from one row to the next in the capture (a row's 21.1 and the panel's spacing of 10).
const ROW_STEP: f32 = 31.1;

/// Adds the zone colours row to the View panel of `session` (a capture of the toolbar's View panel); a session without that panel, or one that already
/// has the row, is left as it is.
pub(crate) fn add_zone_colors_row(session: &mut RuntimeSession) {
    for canvas in &mut session.canvases {
        if add_to(canvas) {
            return;
        }
    }
}

fn add_to(node: &mut Node) -> bool {
    if node.path.ends_with("/ViewButton/VisibilityPanel") {
        if node.children.iter().any(|child| child.name == ZONE_COLORS_ROW) {
            return true;
        }
        // The first row of the panel is its top one ("Fertility"): the new row is its copy one step higher.
        let Some(template) = node.children.first() else {
            return true;
        };
        let mut row = template.clone();
        let from = format!("{}/{}", node.path, template.name);
        let to = format!("{}/{ZONE_COLORS_ROW}", node.path);
        retarget(&mut row, &from, &to);
        row.name = ZONE_COLORS_ROW.to_owned();
        row.sibling_index = 0;
        for child in &mut node.children {
            child.sibling_index += 1;
        }
        node.children.insert(0, row);
        // The panel hangs from the button with its bottom fixed: the extra row makes it taller upwards. (The bounding boxes are in the capture's screen space,
        // y up from the bottom of the screen.)
        if let Some([x0, y0, x1, y1]) = node.native_rect {
            node.native_rect = Some([x0, y0, x1, y1 + ROW_STEP]);
        }
        node.rect.size_delta.y += ROW_STEP;
        return true;
    }
    node.children.iter_mut().any(add_to)
}

/// The name of the gene selector tower's button of the placer panel (the hit code looks for it).
pub const GENE_SELECTOR_BUTTON: &str = "GeneSelector";
/// The name of the hydrothermal vent tower's button of the placer panel (the hit code looks for it).
pub const VENT_BUTTON: &str = "VentTower";
/// The name of the force tower's button of the placer panel (the hit code looks for it).
pub const FORCE_BUTTON: &str = "ForceTower";
/// The name of the whirlpool tower's button of the placer panel (the hit code looks for it).
pub const WHIRL_BUTTON: &str = "WhirlTower";
/// The distance from one button of the placer panel to the next in the capture (a button's 40 and the gap of 5).
const PLACER_STEP: f32 = 45.0;

/// Adds the gene selector tower's button to the placer panel of `session` (a capture of the bottom-left toolbar's placer panel): a copy of the colour selector's
/// button above the highest one, with the DNA icon on the tower's base disc. The panel grows by one button at its top. A session without that panel, or one
/// that already has the button, is left as it is.
pub(crate) fn add_gene_selector_button(session: &mut RuntimeSession) {
    add_placer_button(session, GENE_SELECTOR_BUTTON, "ColorSelector", "dna_icon", None);
}

/// Adds the hydrothermal vent tower's button to the placer panel of `session`: a copy of the radiation tower's button above the highest one, with the plant pellet
/// icon (the pellet placer's) on the tower's base disc. Added after the gene selector's, it goes above it.
pub(crate) fn add_vent_button(session: &mut RuntimeSession) {
    add_placer_button(session, VENT_BUTTON, "RadTower", "plant_paint_tool", Some(Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }));
}

/// Adds the force tower's button to the placer panel of `session`: a copy of the radiation tower's button above the highest one, with the game's light round icon
/// tinted the blue of a pulling force tower's core (the game's own icons for such things are dark and would not show on the tower's dark base disc). Added after the
/// vent's, it goes above it.
pub(crate) fn add_force_button(session: &mut RuntimeSession) {
    add_placer_button(session, FORCE_BUTTON, "RadTower", "base_circle_icon", Some(Color { r: 0.45, g: 0.78, b: 1.0, a: 1.0 }));
}

/// Adds the whirlpool tower's button to the placer panel of `session`: a copy of the radiation tower's button above the highest one, with the game's light round icon
/// tinted the teal of a whirlpool tower's ring. Added after the force tower's, it goes above it.
pub(crate) fn add_whirl_button(session: &mut RuntimeSession) {
    add_placer_button(session, WHIRL_BUTTON, "RadTower", "base_circle_icon", Some(Color { r: 0.4, g: 0.88, b: 0.8, a: 1.0 }));
}

fn add_placer_button(session: &mut RuntimeSession, name: &str, template_name: &str, icon: &str, tint: Option<Color>) {
    for canvas in &mut session.canvases {
        if add_button_to(canvas, name, template_name, icon, tint) {
            return;
        }
    }
}

fn add_button_to(node: &mut Node, name: &str, template_name: &str, icon: &str, tint: Option<Color>) -> bool {
    if node.path.ends_with("/PlacerButton/Placers") {
        if node.children.iter().any(|child| child.name == name) {
            return true;
        }
        let (Some(template), Some(top)) = (
            node.children.iter().find(|child| child.name == template_name),
            node.children.iter().filter_map(|child| child.native_rect).map(|rect| rect[3]).reduce(f32::max),
        ) else {
            return true;
        };
        let Some([_, template_y0, _, _]) = template.native_rect else {
            return true;
        };
        let mut button = template.clone();
        let from = format!("{}/{}", node.path, template.name);
        let to = format!("{}/{name}", node.path);
        rename_paths(&mut button, &from, &to);
        // One button above the highest one (the buttons' y is up: the panel's top is where its biggest y is).
        shift_by(&mut button, top + (PLACER_STEP - 40.0) - template_y0);
        button.name = name.to_owned();
        button.sibling_index = node.children.iter().map(|child| child.sibling_index).max().unwrap_or(0) + 1;
        // The icon: the tower's base disc with the icon over it, in place of the template's own (the colour selector's crystal is drawn by a shader).
        if let Some(disc) = button.children.first_mut() {
            if let Some(inner) = disc.children.first_mut() {
                if let Some([x0, y0, x1, y1]) = inner.native_rect {
                    let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
                    inner.native_rect = Some([cx - 12.0, cy - 12.0, cx + 12.0, cy + 12.0]);
                }
                let image = ImageComponent {
                    sprite: Some(SpriteRef { name: icon.to_owned(), kind: "sprite".to_owned(), ..SpriteRef::default() }),
                    color: tint.or_else(|| inner.images.first().and_then(|image| image.color)),
                    image_type: "Simple".to_owned(),
                    ..ImageComponent::default()
                };
                inner.images = vec![image];
            }
        }
        // The panel hangs from its button with its bottom fixed: one more button makes it taller upwards.
        if let Some([x0, y0, x1, y1]) = node.native_rect {
            node.native_rect = Some([x0, y0, x1, y1 + PLACER_STEP]);
        }
        node.rect.size_delta.y += PLACER_STEP;
        node.children.push(button);
        return true;
    }
    node.children.iter_mut().any(|child| add_button_to(child, name, template_name, icon, tint))
}

/// Gives the copy its own paths.
fn rename_paths(node: &mut Node, from: &str, to: &str) {
    if let Some(rest) = node.path.strip_prefix(from) {
        node.path = format!("{to}{rest}");
    }
    for child in &mut node.children {
        rename_paths(child, from, to);
    }
}

/// Moves a node and everything under it up by `dy` (the bounding boxes are in the capture's screen space, y up).
fn shift_by(node: &mut Node, dy: f32) {
    if let Some([x0, y0, x1, y1]) = node.native_rect {
        node.native_rect = Some([x0, y0 + dy, x1, y1 + dy]);
    }
    for child in &mut node.children {
        shift_by(child, dy);
    }
}

/// Gives the copied row its own paths, its place one row up and its label.
fn retarget(node: &mut Node, from: &str, to: &str) {
    if let Some(rest) = node.path.strip_prefix(from) {
        node.path = format!("{to}{rest}");
    }
    if let Some([x0, y0, x1, y1]) = node.native_rect {
        node.native_rect = Some([x0, y0 + ROW_STEP, x1, y1 + ROW_STEP]);
    }
    if node.name == "Label" {
        for text in &mut node.texts {
            text.text = ZONE_COLORS_LABEL.to_owned();
        }
    }
    for child in &mut node.children {
        retarget(child, from, to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, path: &str, bbox: [f32; 4]) -> Node {
        Node { name: name.to_owned(), path: path.to_owned(), native_rect: Some(bbox), ..Node::default() }
    }

    /// A View panel of two rows (each a toggle and a label), as the capture has them: the rows are 21.1 high, 10 apart, the panel pads 10.
    fn session() -> RuntimeSession {
        let panel_path = "UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/ViewButton/VisibilityPanel";
        let mut panel = node("VisibilityPanel", panel_path, [130.0, 65.0, 410.0, 125.2]);
        panel.rect.size_delta.y = 60.2;
        for (index, name) in ["Fertility", "Pheromones"].into_iter().enumerate() {
            let y0 = 94.1 - index as f32 * 31.1;
            let row_path = format!("{panel_path}/{name}");
            let mut row = node(name, &row_path, [140.0, y0, 389.0, y0 + 21.1]);
            row.sibling_index = index as i32;
            row.children.push(node("Toggle", &format!("{row_path}/Toggle"), [140.0, y0, 160.0, y0 + 20.0]));
            let mut label = node("Label", &format!("{row_path}/Label"), [170.0, y0, 389.0, y0 + 21.1]);
            label.texts.push(crate::ui_runtime::TextComponent { text: name.to_owned(), ..Default::default() });
            row.children.push(label);
            panel.children.push(row);
        }
        let mut canvas = node("UICanvas", "UICanvas", [0.0, 0.0, 1920.0, 1080.0]);
        canvas.children.push(panel);
        RuntimeSession { canvases: vec![canvas], ..RuntimeSession::default() }
    }

    fn panel(session: &RuntimeSession) -> &Node {
        &session.canvases[0].children[0]
    }

    #[test]
    fn the_row_is_the_top_rows_copy_one_step_higher_with_its_own_paths_and_label_and_the_panel_grows_upwards() {
        let mut session = session();
        add_zone_colors_row(&mut session);
        let panel = panel(&session);
        assert_eq!(panel.children.len(), 3);
        let names: Vec<&str> = panel.children.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, [ZONE_COLORS_ROW, "Fertility", "Pheromones"]);
        assert_eq!(panel.children.iter().map(|row| row.sibling_index).collect::<Vec<_>>(), [0, 1, 2]);
        // The new row sits one step above the old top one, the old rows stay where they were.
        let new = &panel.children[0];
        let old = &panel.children[1];
        let (n, o) = (new.native_rect.unwrap(), old.native_rect.unwrap());
        assert!((n[1] - (o[1] + ROW_STEP)).abs() < 1e-3 && (n[3] - (o[3] + ROW_STEP)).abs() < 1e-3);
        assert_eq!(old.native_rect.unwrap(), [140.0, 94.1, 389.0, 115.2]);
        // Every node of the new row has the new paths (a hit or a check mark finds it by them) and the label says what the switch is.
        assert!(new.path.ends_with("/VisibilityPanel/ZoneColors"));
        assert!(new.children.iter().all(|child| child.path.contains("/VisibilityPanel/ZoneColors/")));
        assert_eq!(new.children[1].texts[0].text, ZONE_COLORS_LABEL);
        assert_eq!(old.children[1].texts[0].text, "Fertility", "the copy did not change the original");
        // The panel grew one step at its top.
        assert_eq!(panel.native_rect.unwrap(), [130.0, 65.0, 410.0, 125.2 + ROW_STEP]);
        assert!((panel.rect.size_delta.y - (60.2 + ROW_STEP)).abs() < 1e-3);
    }

    /// A placer panel as the capture has it: five 40 high buttons 5 apart, the highest at the top (y is up), each with an icon, the colour selector's with a
    /// sprite-less image inside it.
    fn placer_session() -> RuntimeSession {
        let panel_path = "UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/PlacerButton/Placers";
        let mut panel = node("Placers", panel_path, [10.0, 65.0, 60.0, 285.0]);
        panel.rect.size_delta.y = 220.0;
        for (index, name) in ["LoadBibite", "PlacePellets", "ColorSelector", "PheroTower", "RadTower"].into_iter().enumerate() {
            let y0 = 65.0 + index as f32 * 45.0;
            let path = format!("{panel_path}/{name}");
            let mut button = node(name, &path, [15.0, y0, 55.0, y0 + 40.0]);
            button.sibling_index = index as i32;
            let mut icon = node("Image", &format!("{path}/Image"), [20.0, y0 + 5.0, 50.0, y0 + 35.0]);
            icon.images.push(ImageComponent { sprite: Some(SpriteRef { name: "laser_circle".to_owned(), ..SpriteRef::default() }), ..ImageComponent::default() });
            if name == "ColorSelector" {
                icon.children.push(node("Image", &format!("{path}/Image/Image"), [27.9, y0 + 12.9, 42.1, y0 + 27.1]));
            }
            if name == "RadTower" {
                let mut inner = node("Image", &format!("{path}/Image/Image"), [20.0, y0 + 5.0, 50.0, y0 + 35.0]);
                inner.images.push(ImageComponent { sprite: Some(SpriteRef { name: "radioactive".to_owned(), ..SpriteRef::default() }), color: Some(Color { r: 0.5, g: 0.58, b: 0.0, a: 1.0 }), ..ImageComponent::default() });
                icon.children.push(inner);
            }
            button.children.push(icon);
            panel.children.push(button);
        }
        let mut canvas = node("UICanvas", "UICanvas", [0.0, 0.0, 1920.0, 1080.0]);
        canvas.children.push(panel);
        RuntimeSession { canvases: vec![canvas], ..RuntimeSession::default() }
    }

    #[test]
    fn the_gene_selector_button_is_the_colour_selectors_copy_above_the_highest_button_with_the_dna_icon_and_the_panel_grows_upwards() {
        let mut session = placer_session();
        add_gene_selector_button(&mut session);
        let panel = &session.canvases[0].children[0];
        let names: Vec<&str> = panel.children.iter().map(|button| button.name.as_str()).collect();
        assert_eq!(names, ["LoadBibite", "PlacePellets", "ColorSelector", "PheroTower", "RadTower", GENE_SELECTOR_BUTTON]);
        let (new, highest) = (&panel.children[5], &panel.children[4]);
        let (n, h) = (new.native_rect.unwrap(), highest.native_rect.unwrap());
        assert!((n[1] - h[1] - PLACER_STEP).abs() < 1e-3 && (n[3] - h[3] - PLACER_STEP).abs() < 1e-3, "one step above the highest button: {n:?} {h:?}");
        assert_eq!(new.sibling_index, 5);
        // Its own paths, all the way down.
        assert!(new.path.ends_with("/Placers/GeneSelector") && new.children[0].path.ends_with("/Placers/GeneSelector/Image"));
        let inner = &new.children[0].children[0];
        assert!(inner.path.ends_with("/Placers/GeneSelector/Image/Image"));
        // The icon over the disc: the DNA sprite, about 24 square, centred where the crystal was.
        assert_eq!(inner.images[0].sprite.as_ref().map(|sprite| sprite.name.as_str()), Some("dna_icon"));
        let [x0, y0, x1, y1] = inner.native_rect.unwrap();
        assert!(((x1 - x0) - 24.0).abs() < 1e-3 && ((y1 - y0) - 24.0).abs() < 1e-3);
        // The colour selector's own button is untouched, and the panel is taller by a step at its top.
        assert!(panel.children[2].children[0].children[0].images.is_empty());
        assert_eq!(panel.native_rect.unwrap(), [10.0, 65.0, 60.0, 285.0 + PLACER_STEP]);
        assert!((panel.rect.size_delta.y - (220.0 + PLACER_STEP)).abs() < 1e-3);
        // Once only; a session without the panel is left as it is.
        let once = format!("{:?}", session.canvases[0].children[0]);
        add_gene_selector_button(&mut session);
        assert_eq!(format!("{:?}", session.canvases[0].children[0]), once);
        let mut other = RuntimeSession { canvases: vec![node("UICanvas", "UICanvas", [0.0; 4])], ..RuntimeSession::default() };
        add_gene_selector_button(&mut other);
        assert!(other.canvases[0].children.is_empty());
    }

    #[test]
    fn the_vent_button_is_the_radiation_towers_copy_above_the_gene_selectors_with_the_plant_pellet_icon_and_the_panel_grows_by_two_buttons() {
        let mut session = placer_session();
        add_gene_selector_button(&mut session);
        add_vent_button(&mut session);
        let panel = &session.canvases[0].children[0];
        let names: Vec<&str> = panel.children.iter().map(|button| button.name.as_str()).collect();
        assert_eq!(names, ["LoadBibite", "PlacePellets", "ColorSelector", "PheroTower", "RadTower", GENE_SELECTOR_BUTTON, VENT_BUTTON]);
        let (vent, gene, rad) = (&panel.children[6], &panel.children[5], &panel.children[4]);
        let (v, g) = (vent.native_rect.unwrap(), gene.native_rect.unwrap());
        assert!((v[1] - g[1] - PLACER_STEP).abs() < 1e-3 && (v[3] - g[3] - PLACER_STEP).abs() < 1e-3, "one step above the gene selector's: {v:?} {g:?}");
        assert_eq!(vent.sibling_index, 6);
        assert!(vent.path.ends_with("/Placers/VentTower") && vent.children[0].path.ends_with("/Placers/VentTower/Image") && vent.children[0].children[0].path.ends_with("/Placers/VentTower/Image/Image"));
        // The icon is the pellet placer's over the disc, about 24 square and white; the radiation tower's own icon is untouched.
        let inner = &vent.children[0].children[0];
        assert_eq!(inner.images[0].sprite.as_ref().map(|sprite| sprite.name.as_str()), Some("plant_paint_tool"));
        let colour = inner.images[0].color.expect("a colour");
        assert_eq!((colour.r, colour.g, colour.b, colour.a), (1.0, 1.0, 1.0, 1.0));
        let [x0, y0, x1, y1] = inner.native_rect.unwrap();
        assert!(((x1 - x0) - 24.0).abs() < 1e-3 && ((y1 - y0) - 24.0).abs() < 1e-3);
        assert_eq!(rad.children[0].children[0].images[0].sprite.as_ref().map(|sprite| sprite.name.as_str()), Some("radioactive"));
        assert_eq!(panel.native_rect.unwrap(), [10.0, 65.0, 60.0, 285.0 + 2.0 * PLACER_STEP]);
        assert!((panel.rect.size_delta.y - (220.0 + 2.0 * PLACER_STEP)).abs() < 1e-3);
        // Once only.
        let once = format!("{:?}", session.canvases[0].children[0]);
        add_vent_button(&mut session);
        assert_eq!(format!("{:?}", session.canvases[0].children[0]), once);
    }

    #[test]
    fn the_force_button_is_the_radiation_towers_copy_above_the_vents_with_a_light_blue_round_icon_and_the_panel_grows_by_one_more_button() {
        let mut session = placer_session();
        add_gene_selector_button(&mut session);
        add_vent_button(&mut session);
        add_force_button(&mut session);
        let panel = &session.canvases[0].children[0];
        let names: Vec<&str> = panel.children.iter().map(|button| button.name.as_str()).collect();
        assert_eq!(names, ["LoadBibite", "PlacePellets", "ColorSelector", "PheroTower", "RadTower", GENE_SELECTOR_BUTTON, VENT_BUTTON, FORCE_BUTTON]);
        let (force, vent) = (&panel.children[7], &panel.children[6]);
        let (f, v) = (force.native_rect.unwrap(), vent.native_rect.unwrap());
        assert!((f[1] - v[1] - PLACER_STEP).abs() < 1e-3 && (f[3] - v[3] - PLACER_STEP).abs() < 1e-3, "one step above the vent's: {f:?} {v:?}");
        assert_eq!(force.sibling_index, 7);
        assert!(force.path.ends_with("/Placers/ForceTower") && force.children[0].children[0].path.ends_with("/Placers/ForceTower/Image/Image"));
        assert_eq!(force.children[0].children[0].images[0].sprite.as_ref().map(|sprite| sprite.name.as_str()), Some("base_circle_icon"));
        let colour = force.children[0].children[0].images[0].color.expect("a colour");
        assert!(colour.b > colour.r && colour.b > 0.9, "a blue that is not dark: {colour:?}");
        assert_eq!(panel.native_rect.unwrap(), [10.0, 65.0, 60.0, 285.0 + 3.0 * PLACER_STEP]);
        // Once only.
        let once = format!("{:?}", session.canvases[0].children[0]);
        add_force_button(&mut session);
        assert_eq!(format!("{:?}", session.canvases[0].children[0]), once);
    }

    #[test]
    fn the_whirl_button_is_the_radiation_towers_copy_above_the_force_towers_with_a_teal_round_icon_and_the_panel_grows_by_one_more_button() {
        let mut session = placer_session();
        add_gene_selector_button(&mut session);
        add_vent_button(&mut session);
        add_force_button(&mut session);
        add_whirl_button(&mut session);
        let panel = &session.canvases[0].children[0];
        let names: Vec<&str> = panel.children.iter().map(|button| button.name.as_str()).collect();
        assert_eq!(names, ["LoadBibite", "PlacePellets", "ColorSelector", "PheroTower", "RadTower", GENE_SELECTOR_BUTTON, VENT_BUTTON, FORCE_BUTTON, WHIRL_BUTTON]);
        let (whirl, force) = (&panel.children[8], &panel.children[7]);
        let (w, f) = (whirl.native_rect.unwrap(), force.native_rect.unwrap());
        assert!((w[1] - f[1] - PLACER_STEP).abs() < 1e-3 && (w[3] - f[3] - PLACER_STEP).abs() < 1e-3, "one step above the force tower's: {w:?} {f:?}");
        assert_eq!(whirl.sibling_index, 8);
        assert!(whirl.path.ends_with("/Placers/WhirlTower") && whirl.children[0].children[0].path.ends_with("/Placers/WhirlTower/Image/Image"));
        assert_eq!(whirl.children[0].children[0].images[0].sprite.as_ref().map(|sprite| sprite.name.as_str()), Some("base_circle_icon"));
        let colour = whirl.children[0].children[0].images[0].color.expect("a colour");
        assert!(colour.g > colour.r && colour.g > 0.8, "a teal that is not dark: {colour:?}");
        assert_eq!(panel.native_rect.unwrap(), [10.0, 65.0, 60.0, 285.0 + 4.0 * PLACER_STEP]);
        // Once only.
        let once = format!("{:?}", session.canvases[0].children[0]);
        add_whirl_button(&mut session);
        assert_eq!(format!("{:?}", session.canvases[0].children[0]), once);
    }

    #[test]
    fn a_session_that_has_the_row_or_no_view_panel_is_left_as_it_is() {
        let mut session = session();
        add_zone_colors_row(&mut session);
        let once = format!("{:?}", panel(&session));
        add_zone_colors_row(&mut session);
        assert_eq!(format!("{:?}", panel(&session)), once, "added only once");
        let mut other = RuntimeSession { canvases: vec![node("UICanvas", "UICanvas", [0.0; 4])], ..RuntimeSession::default() };
        add_zone_colors_row(&mut other);
        assert!(other.canvases[0].children.is_empty());
    }
}
