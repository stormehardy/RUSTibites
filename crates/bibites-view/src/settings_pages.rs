//! The extra pages of the Settings screens (so far the Calibration page: see `machine_calibration.rs`). The screens are replayed from the game's captures (a column
//! of buttons at the right: Graphic Settings, Game Settings, Hotkeys, Reset Warnings; the page of the chosen one at the left), so a page is made from the captures' own
//! parts: one more button in the column ([`add_page_buttons`]) and a page for it ([`make_page`]) that is the Game page with its groups taken out and one group of its
//! own put in. The rows are copies of the capture's rows (a check box row; the slider row gives the places of the text and the buttons), and the buttons are copies of the column's button, all in
//! the game's looks. A page is a [`Page`]: a title and six [`Row`]s (the six places of the group that is copied).

use crate::machine_calibration::{LABEL_USE, names};
use crate::ui_runtime::{Node, RuntimeSession};

/// The name of the session of the Calibration page (the other pages' sessions are `main_menu_settings_graphics`, `_gameplay` and `_hotkeys`).
pub const CALIBRATION_SESSION: &str = "main_menu_settings_calibration";
/// The name of the Calibration page's button of the column (the menu's hit code looks for it).
pub const CALIBRATION_BUTTON: &str = "Calibration Button";
/// What the column's button says (the others say "Graphic Settings", "Game Settings" and "Hotkeys").
const CALIBRATION_BUTTON_LABEL: &str = "Calibration";
/// The settings tab of the Calibration page (0 graphics, 1 game, 2 hotkeys).
pub const CALIBRATION_TAB: u8 = 3;
/// The distance from one button of the column to the next in the capture (an 80 high button and a gap of 20).
const BUTTON_STEP: f32 = 100.0;
/// How many rows the group of the capture that is copied has (the Save Settings group), and so how many places a page's rows have.
pub const SLOTS: usize = 6;

/// One row of a page.
#[derive(Clone, Copy, Debug)]
pub enum Row {
    /// A check box row bound to the setting with this label.
    Toggle(&'static str),
    /// A label and a line of live text (the text node has this name).
    Info {
        /// The row's label.
        label: &'static str,
        /// The name of the text node.
        node: &'static str,
    },
    /// A label and one button (its node name and what it says).
    Button {
        /// The row's label.
        label: &'static str,
        /// The button node's name.
        node: &'static str,
        /// What the button says.
        text: &'static str,
    },
}

/// A page: the session it is made as, its title and its rows.
#[derive(Clone, Copy, Debug)]
pub struct Page {
    /// The name of its session.
    pub session: &'static str,
    /// The title of its group.
    pub title: &'static str,
    /// Its rows, top to bottom (at most [`SLOTS`]).
    pub rows: &'static [Row],
}

/// The Calibration page: the check box that says whether the calibration is used, the button that runs the test, and four lines of what it found.
pub const CALIBRATION: Page = Page {
    session: CALIBRATION_SESSION,
    title: "Calibration",
    rows: &[
        Row::Toggle(LABEL_USE),
        Row::Button { label: "Calibration Test", node: names::RUN, text: "Run Test" },
        Row::Info { label: "Status", node: names::STATUS },
        Row::Info { label: "Machine", node: names::MACHINE },
        Row::Info { label: "Multi-Core", node: names::MULTICORE },
        Row::Info { label: "Graphics", node: names::GRAPHICS },
    ],
};

/// Every extra page.
pub const PAGES: [&Page; 1] = [&CALIBRATION];

/// The page that is made as the session called `name`, if it is one of the extra pages.
#[must_use]
pub fn page_named(name: &str) -> Option<&'static Page> {
    PAGES.into_iter().find(|page| page.session == name)
}

// ---------------------------------------------------------------------------------------------------------------------------------------------------------
// The buttons in the column
// ---------------------------------------------------------------------------------------------------------------------------------------------------------

/// Adds the Calibration button under the Hotkeys one in the column of `session` (a capture of a Settings page); a session without the column, or one that has the
/// button, is left as it is.
pub(crate) fn add_page_buttons(session: &mut RuntimeSession) {
    for canvas in &mut session.canvases {
        if add_buttons_to(canvas) {
            return;
        }
    }
}

fn add_buttons_to(node: &mut Node) -> bool {
    if node.path.ends_with("/SettingsPanel/SelectorPanel/List") {
        add_button_after(node, "Controls Button", CALIBRATION_BUTTON, CALIBRATION_BUTTON_LABEL, 1.0);
        return true;
    }
    node.children.iter_mut().any(add_buttons_to)
}

/// Puts a copy of the Hotkeys button, `steps` buttons lower, right after the button `after` of the column, named `name` and saying `label`.
fn add_button_after(list: &mut Node, after: &str, name: &str, label: &str, steps: f32) {
    if list.children.iter().any(|child| child.name == name) {
        return;
    }
    let (Some(at), Some(template)) =
        (list.children.iter().position(|child| child.name == after), list.children.iter().find(|child| child.name == "Controls Button"))
    else {
        return;
    };
    let mut button = template.clone();
    let from = format!("{}/{}", list.path, template.name);
    let to = format!("{}/{name}", list.path);
    retarget(&mut button, &from, &to);
    shift(&mut button, 0.0, -BUTTON_STEP * steps);
    button.name = name.to_owned();
    button.sibling_index = list.children[at].sibling_index + 1;
    set_all_texts(&mut button, label);
    for later in list.children.iter_mut().skip(at + 1) {
        later.sibling_index += 1;
    }
    list.children.insert(at + 1, button);
}

// ---------------------------------------------------------------------------------------------------------------------------------------------------------
// The pages
// ---------------------------------------------------------------------------------------------------------------------------------------------------------

/// Turns `session`, a capture of the Game page, into `page`: the Game page's groups go, and the one group of the page's rows takes their place. Returns whether the
/// page could be made (the capture has the parts it is made of).
pub(crate) fn make_page(session: &mut RuntimeSession, page: &Page) -> bool {
    let Some(button) = find_by_suffix(&session.canvases, "/SettingsPanel/SelectorPanel/List/Reset Warnings").cloned() else {
        return false;
    };
    session.canvases.iter_mut().any(|canvas| page_in(canvas, page, &button))
}

fn page_in(node: &mut Node, page: &Page, button: &Node) -> bool {
    if node.path.ends_with("/PagesPanels/Game/Scroll View/Viewport/Content/GameHolder") {
        let Some(group) = node.children.first().and_then(|save| group_of(save, page, button)) else {
            return false;
        };
        node.children = vec![group];
        return true;
    }
    node.children.iter_mut().any(|child| page_in(child, page, button))
}

fn find_by_suffix<'a>(nodes: &'a [Node], suffix: &str) -> Option<&'a Node> {
    nodes.iter().find_map(|node| {
        if node.path.ends_with(suffix) {
            Some(node)
        } else {
            find_by_suffix(&node.children, suffix)
        }
    })
}

/// The row prototypes and the places in the row that a page's rows are made from.
struct Parts {
    toggle: Node,
    name_end: f32,
    control_start: f32,
    value_start: f32,
}

/// The group of a page's rows: the Save Settings group of the capture (a title and six rows), with the rows made over.
fn group_of(save: &Node, page: &Page, button: &Node) -> Option<Node> {
    let mut group = save.clone();
    set_label(group.children.iter_mut().find(|child| child.name == "Text (TMP)")?, page.title);
    let holder = group.children.iter_mut().find(|child| child.name == "SettingsHolder")?;
    let slots: Vec<[f32; 4]> = holder.children.iter().filter_map(|row| row.native_rect).collect();
    if slots.len() < SLOTS {
        return None;
    }
    let first = |prefix: &str| holder.children.iter().find(|row| row.name.starts_with(prefix)).cloned();
    let toggle = first("SettingToggle")?;
    let slider = first("SettingSlider")?;
    // Where a row's parts sit in the slider row: the name takes the left, the control (slider, here the buttons or the text) the right.
    let parts = Parts {
        name_end: child(&slider, "Name")?.native_rect?[2],
        control_start: child(&slider, "NormalSecton")?.native_rect?[0],
        value_start: child(&slider, "FloatInfoText")?.native_rect?[0],
        toggle,
    };
    let mut rows: Vec<Node> = Vec::with_capacity(SLOTS);
    for (slot, row) in page.rows.iter().take(SLOTS).enumerate() {
        rows.push(make_row(row, slots[slot], &parts, button)?);
    }
    for (index, row) in rows.iter_mut().enumerate() {
        row.sibling_index = index as i32;
    }
    holder.children = rows;
    Some(group)
}

/// One row of a page, in the place `slot`.
fn make_row(row: &Row, slot: [f32; 4], parts: &Parts, button: &Node) -> Option<Node> {
    Some(match *row {
        Row::Toggle(label) => row_at(&parts.toggle, slot, label),
        Row::Info { label, node } => {
            // The name at the left, the live text over the rest of the row.
            let mut row = plain_row(parts, slot, label);
            let [_, y0, right, y1] = row.native_rect?;
            let text = child(&row, "Name")?.clone();
            row.children.push(made_from(&text, node, [parts.value_start, y0, right, y1], ""));
            row
        }
        Row::Button { label, node, text } => {
            let mut row = plain_row(parts, slot, label);
            let [_, y0, right, y1] = row.native_rect?;
            let made = button_from(button, &row.path, node, text, [parts.control_start, y0, right, y1]);
            row.children.push(made);
            row
        }
    })
}

/// A row with a name and nothing else (a check box row without its box, its name as short as a slider row's): what the rows with text and buttons start from.
fn plain_row(parts: &Parts, slot: [f32; 4], label: &str) -> Node {
    let mut row = row_at(&parts.toggle, slot, label);
    narrow_name(&mut row, parts.name_end);
    row.children.retain(|part| part.name != "Toggle");
    row
}

/// A copy of `template` (a row of the capture) moved to the place `slot` (the bounding box of a row of the group) and named `label`.
fn row_at(template: &Node, slot: [f32; 4], label: &str) -> Node {
    let mut row = template.clone();
    let dy = slot[1] - template.native_rect.map_or(slot[1], |rect| rect[1]);
    shift(&mut row, 0.0, dy);
    if let Some(name) = row.children.iter_mut().find(|part| part.name == "Name") {
        set_label(name, label);
    }
    row
}

/// The name of a row ends where the capture's slider rows' names end (a check box row's name runs on to its box).
fn narrow_name(row: &mut Node, end: f32) {
    if let Some(name) = row.children.iter_mut().find(|part| part.name == "Name")
        && let Some(rect) = name.native_rect.as_mut()
    {
        rect[2] = end;
    }
}

/// A text node made from `template` (a row's name) at `rect`, named `name`, saying `text`.
fn made_from(template: &Node, name: &str, rect: [f32; 4], text: &str) -> Node {
    let mut node = template.clone();
    let parent = node.path.rsplit_once('/').map_or("", |(parent, _)| parent).to_owned();
    node.path = format!("{parent}/{name}");
    node.name = name.to_owned();
    node.native_rect = Some(rect);
    set_all_texts(&mut node, text);
    node
}

/// A button made from the column's `button` (the capture's one: its picture, its text), at `rect` in the row at `row_path`, named `name` and saying `label`.
fn button_from(button: &Node, row_path: &str, name: &str, label: &str, rect: [f32; 4]) -> Node {
    let mut node = button.clone();
    let from = node.path.clone();
    let to = format!("{row_path}/{name}");
    retarget(&mut node, &from, &to);
    node.name = name.to_owned();
    node.native_rect = Some(rect);
    // The label fills the button; a row is 40 high, so the text is the size of the rows' own.
    for part in &mut node.children {
        part.native_rect = Some(rect);
        for text in &mut part.texts {
            text.text = label.to_owned();
            text.font_size = 25.0;
        }
    }
    set_all_texts(&mut node, label);
    node
}

fn child<'a>(node: &'a Node, name: &str) -> Option<&'a Node> {
    node.children.iter().find(|part| part.name == name)
}

/// Gives the copy its own paths: the ones that start with `from` start with `to`.
fn retarget(node: &mut Node, from: &str, to: &str) {
    if let Some(rest) = node.path.strip_prefix(from) {
        node.path = format!("{to}{rest}");
    }
    for part in &mut node.children {
        retarget(part, from, to);
    }
}

/// Moves a node and everything under it by (`dx`, `dy`) (the bounding boxes are in the capture's screen space, y up).
fn shift(node: &mut Node, dx: f32, dy: f32) {
    if let Some([x0, y0, x1, y1]) = node.native_rect {
        node.native_rect = Some([x0 + dx, y0 + dy, x1 + dx, y1 + dy]);
    }
    for part in &mut node.children {
        shift(part, dx, dy);
    }
}

/// Every text of `node` (and of what is under it) says `text`.
fn set_all_texts(node: &mut Node, text: &str) {
    for own in &mut node.texts {
        own.text = text.to_owned();
    }
    for part in &mut node.children {
        set_all_texts(part, text);
    }
}

/// The text of `node` itself.
fn set_label(node: &mut Node, text: &str) {
    for own in &mut node.texts {
        own.text = text.to_owned();
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_runtime::TextComponent;

    fn node(name: &str, path: &str, rect: Option<[f32; 4]>) -> Node {
        Node { name: name.to_owned(), path: path.to_owned(), native_rect: rect, ..Node::default() }
    }

    fn texted(mut node: Node, text: &str) -> Node {
        node.texts.push(TextComponent { text: text.to_owned(), ..TextComponent::default() });
        node
    }

    /// The parts of a Settings capture that the pages are made from: the column of buttons and the Game page with a group of six rows (a check box, a dropdown,
    /// three check boxes and a slider, as the Save Settings group has them) and one more group.
    fn session() -> RuntimeSession {
        let column = "UI/SettingsPanel/SelectorPanel/List";
        let mut list = node("List", column, None);
        for (index, (name, label)) in
            [("GraphicsButton", "Graphic Settings"), ("Gameplay Button", "Game Settings"), ("Controls Button", "Hotkeys"), ("spacer", ""), ("Reset Warnings", "Reset Warnings")]
                .into_iter()
                .enumerate()
        {
            let y0 = 775.0 - index as f32 * 100.0;
            let path = format!("{column}/{name}");
            let mut button = node(name, &path, Some([1379.0, y0, 1855.0, y0 + 80.0]));
            button.sibling_index = index as i32;
            button.children.push(texted(node("Text (TMP)", &format!("{path}/Text (TMP)"), Some([1389.0, y0, 1845.0, y0 + 80.0])), label));
            list.children.push(button);
        }
        let holder_path = "UI/SettingsPanel/PagesPanels/Game/Scroll View/Viewport/Content/GameHolder";
        let group_path = format!("{holder_path}/SettingGroupPrefab(Clone)");
        let mut group = node("SettingGroupPrefab(Clone)", &group_path, Some([115.0, 535.0, 1222.0, 965.0]));
        group.children.push(texted(node("Text (TMP)", &format!("{group_path}/Text (TMP)"), Some([115.0, 915.0, 1222.0, 965.0])), "Save Settings"));
        let rows_path = format!("{group_path}/SettingsHolder");
        let mut rows = node("SettingsHolder", &rows_path, Some([115.0, 535.0, 1222.0, 905.0]));
        for slot in 0..SLOTS {
            let y0 = 855.0 - slot as f32 * 60.0;
            let kind = match slot {
                1 => "SettingDropdown(Clone)",
                s if s == SLOTS - 1 => "SettingSlider(Clone)",
                _ => "SettingToggle(Clone)",
            };
            let path = format!("{rows_path}/{kind}");
            let mut row = node(kind, &path, Some([115.0, y0, 1222.0, y0 + 40.0]));
            row.children.push(texted(node("Name", &format!("{path}/Name"), Some([115.0, y0, 857.0, y0 + 40.0])), &format!("Row {slot}")));
            if slot == SLOTS - 1 {
                row.children.last_mut().unwrap().native_rect = Some([115.0, y0, 548.0, y0 + 40.0]);
                row.children.push(texted(node("FloatInfoText", &format!("{path}/FloatInfoText"), Some([563.0, y0, 798.0, y0 + 40.0])), "2k"));
                row.children.push(node("NormalSecton", &format!("{path}/NormalSecton"), Some([813.0, y0, 1222.0, y0 + 40.0])));
            } else if slot == 1 {
                row.children.push(node("Dropdown", &format!("{path}/Dropdown"), Some([968.0, y0, 1165.0, y0 + 40.0])));
            } else {
                row.children.push(node("Toggle", &format!("{path}/Toggle"), Some([872.0, y0, 1222.0, y0 + 40.0])));
            }
            rows.children.push(row);
        }
        group.children.push(rows);
        let mut game_holder = node("GameHolder", holder_path, None);
        game_holder.children.push(group.clone());
        game_holder.children.push(group);
        let mut canvas = node("UI", "UI", None);
        canvas.children.push(list);
        canvas.children.push(game_holder);
        RuntimeSession { canvases: vec![canvas], ..RuntimeSession::default() }
    }

    fn list(session: &RuntimeSession) -> &Node {
        &session.canvases[0].children[0]
    }

    fn rows(session: &RuntimeSession) -> &[Node] {
        let group = &session.canvases[0].children[1].children[0];
        &group.children[1].children
    }

    fn label_of(row: &Node) -> &str {
        row.children.iter().find(|part| part.name == "Name").unwrap().texts[0].text.as_str()
    }

    #[test]
    fn the_calibration_button_is_a_copy_of_the_hotkeys_button_one_step_lower_with_its_own_paths_and_label() {
        let mut session = session();
        add_page_buttons(&mut session);
        let list = list(&session);
        let names: Vec<&str> = list.children.iter().map(|button| button.name.as_str()).collect();
        assert_eq!(names, ["GraphicsButton", "Gameplay Button", "Controls Button", CALIBRATION_BUTTON, "spacer", "Reset Warnings"]);
        assert_eq!(list.children.iter().map(|button| button.sibling_index).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5], "the ones after it moved down");
        let hotkeys = list.children[2].native_rect.unwrap();
        let button = &list.children[3];
        let rect = button.native_rect.unwrap();
        assert!((hotkeys[1] - rect[1] - BUTTON_STEP).abs() < 1e-3 && (hotkeys[3] - rect[3] - BUTTON_STEP).abs() < 1e-3);
        assert!(button.path.ends_with(&format!("/List/{}", button.name)) && button.children[0].path.ends_with(&format!("/List/{}/Text (TMP)", button.name)));
        assert_eq!(button.children[0].texts[0].text, CALIBRATION_BUTTON_LABEL);
        assert_eq!(list.children[2].children[0].texts[0].text, "Hotkeys", "the copy left the original alone");
        // Added once only; a session without the column is left as it is.
        let before = format!("{:?}", list.children);
        add_page_buttons(&mut session);
        assert_eq!(format!("{:?}", self::list(&session).children), before);
        let mut other = RuntimeSession { canvases: vec![node("UI", "UI", None)], ..RuntimeSession::default() };
        add_page_buttons(&mut other);
        assert!(other.canvases[0].children.is_empty());
    }

    #[test]
    fn the_calibration_page_is_one_group_of_six_rows_a_check_box_a_button_and_four_lines_in_the_places_of_the_captures_rows() {
        let mut session = session();
        assert!(make_page(&mut session, &CALIBRATION));
        let holder = &session.canvases[0].children[1];
        assert_eq!(holder.children.len(), 1, "the other groups of the Game page are gone");
        assert_eq!(holder.children[0].children[0].texts[0].text, "Calibration");
        let rows = rows(&session);
        assert_eq!(rows.len(), SLOTS);
        let labels: Vec<&str> = rows.iter().map(label_of).collect();
        assert_eq!(labels, ["Use Calibration", "Calibration Test", "Status", "Machine", "Multi-Core", "Graphics"]);
        for (slot, row) in rows.iter().enumerate() {
            let y0 = 855.0 - slot as f32 * 60.0;
            assert_eq!(row.native_rect.unwrap(), [115.0, y0, 1222.0, y0 + 40.0], "row {slot}");
            assert_eq!(row.sibling_index, slot as i32);
        }
        // The first is a check box row: the menu binds it to its setting by its name.
        assert!(rows[0].name.starts_with("SettingToggle") && rows[0].children.iter().any(|part| part.name == "Toggle"));
        // The test button: one button over the control column, saying what it does.
        let run = rows[1].children.iter().find(|part| part.name == names::RUN).expect("the button");
        let y0 = 855.0 - 60.0;
        assert_eq!(run.native_rect.unwrap(), [813.0, y0, 1222.0, y0 + 40.0]);
        assert_eq!(run.children[0].texts[0].text, "Run Test");
        assert!(run.path.ends_with("/SettingToggle(Clone)/CalibrationRun") && run.children[0].path.ends_with("/CalibrationRun/Text (TMP)"));
        // The four lines: the name at the left, the live text from where the slider rows' values start to the row's end.
        for (row, node) in [(2, names::STATUS), (3, names::MACHINE), (4, names::MULTICORE), (5, names::GRAPHICS)] {
            assert!(rows[row].children.iter().all(|part| part.name != "Toggle"), "no box in a line of text");
            let text = rows[row].children.iter().find(|part| part.name == node).expect("the live text");
            let y0 = 855.0 - row as f32 * 60.0;
            assert_eq!(text.native_rect.unwrap(), [563.0, y0, 1222.0, y0 + 40.0], "{node}");
            let name = rows[row].children.iter().find(|part| part.name == "Name").unwrap();
            assert_eq!(name.native_rect.unwrap()[2], 548.0, "the name stops where the slider rows' names stop");
        }
    }

    #[test]
    fn the_page_is_found_by_its_session_name() {
        assert_eq!(page_named(CALIBRATION_SESSION).map(|page| page.title), Some("Calibration"));
        assert!(page_named("main_menu_settings_gameplay").is_none());
    }

    #[test]
    fn a_capture_without_the_parts_gives_no_page_and_no_change() {
        let mut empty = RuntimeSession { canvases: vec![node("UI", "UI", None)], ..RuntimeSession::default() };
        assert!(!make_page(&mut empty, &CALIBRATION));
        let mut no_game_page = session();
        no_game_page.canvases[0].children.pop();
        assert!(!make_page(&mut no_game_page, &CALIBRATION));
    }
}
