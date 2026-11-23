//! RUSTibites Improved: the extra pages of the Settings screens (Music, Species Names). The screens are replayed from the game's captures (a column of buttons at the
//! right: Graphic Settings, Game Settings, Hotkeys, Reset Warnings; the page of the chosen one at the left), so the pages are made from the captures' own parts: more
//! buttons in the column ([`add_page_buttons`]) and a page for each ([`make_page`]) that is the Game page with its groups taken out and one group of its own put in. The
//! rows are copies of the capture's rows (a check box row, a slider row, a dropdown row), and the buttons are copies of the column's button, all in the game's looks.
//! A page is a [`Page`]: a title and six [`Row`]s (the six places of the group that is copied).

use crate::machine_calibration::{LABEL_USE, names};
use crate::ui_runtime::{Node, RuntimeSession};

/// The name of the session of the Music page (the other pages' sessions are `main_menu_settings_graphics`, `_gameplay` and `_hotkeys`).
pub const MUSIC_SESSION: &str = "main_menu_settings_music";
/// The name of the session of the Species Names page.
pub const NAMES_SESSION: &str = "main_menu_settings_names";
/// The name of the session of the Towers page (it began as the gene selector's page: the names of its parts say Gene).
pub const GENE_SESSION: &str = "main_menu_settings_gene";
/// The name of the session of the Extra Hotkeys page.
pub const KEYS_SESSION: &str = "main_menu_settings_keys";
/// The label of the setting the Towers page shows for the gene selector towers (`Feature::GeneSelectorTower`).
pub const LABEL_GENE_TOWERS: &str = "Gene Selector Towers";
/// The label of the setting the Towers page shows for the hydrothermal vents (`Feature::HydrothermalVent`).
pub const LABEL_VENTS: &str = "Hydrothermal Vents";
/// The label of the setting the Towers page shows for the force towers (`Feature::ForceTower`).
pub const LABEL_FORCE: &str = "Force Towers";
/// The label of the setting the Towers page shows for the whirlpool towers (`Feature::Whirlpool`).
pub const LABEL_WHIRL: &str = "Whirlpool Towers";
/// The label of the setting the Towers page shows for the streams of the Zones tab (`Feature::Streams`).
pub const LABEL_STREAMS: &str = "Streams";
/// The name of the Music page's button of the column (the menu's hit code looks for it).
pub const MUSIC_BUTTON: &str = "Music Button";
/// The name of the Species Names page's button of the column.
pub const NAMES_BUTTON: &str = "Names Button";
/// What the column's buttons say (the others say "Graphic Settings", "Game Settings" and "Hotkeys").
const MUSIC_BUTTON_LABEL: &str = "Music Settings";
const NAMES_BUTTON_LABEL: &str = "Species Names";
/// The name of the Towers page's button of the column, and what it says.
pub const GENE_BUTTON: &str = "Gene Button";
const GENE_BUTTON_LABEL: &str = "Towers & Streams";
/// The name of the Extra Hotkeys page's button of the column, and what it says.
pub const KEYS_BUTTON: &str = "Keys Button";
const KEYS_BUTTON_LABEL: &str = "Extra Hotkeys";
/// The name of the session of the Calibration page (`machine_calibration.rs`: the test of the machine and what it found).
pub const CALIBRATION_SESSION: &str = "main_menu_settings_calibration";
/// The name of the Calibration page's button of the column, and what it says.
pub const CALIBRATION_BUTTON: &str = "Calibration Button";
const CALIBRATION_BUTTON_LABEL: &str = "Calibration";
/// The settings tab of the Calibration page (0 graphics, 1 game, 2 hotkeys, 3 music, 4 species names, 5 towers, 6 extra hotkeys).
pub const CALIBRATION_TAB: u8 = 7;
/// The names of the Extra Hotkeys page's two buttons (the menu's hit code looks for them; the first one says the key, live).
pub const KEY_HIDE_OBJECTS: &str = "KeyHideObjects";
/// See [`KEY_HIDE_OBJECTS`].
pub const KEYS_RESET: &str = "KeysReset";
/// The names of the Extra Hotkeys page's lines of text: two that always say the same thing (see [`static_line`]), and the one that says what to do or what just happened.
pub const KEYS_INFO_HIDES: &str = "KeysInfoHides";
/// See [`KEYS_INFO_HIDES`].
pub const KEYS_INFO_STAYS: &str = "KeysInfoStays";
/// See [`KEYS_INFO_HIDES`].
pub const KEYS_INFO_STATE: &str = "KeysInfoState";
/// The name of the Towers page's line of text (it says the same thing always: see [`static_line`]: where the towers are placed from and that they are kept beside the save).
pub const GENE_INFO_PLACE: &str = "GeneInfoPlace";
/// The name of the text that shows what is playing (the menu's live text code looks for it).
pub const NOW_PLAYING: &str = "NowPlaying";
/// The names of the Music page's three buttons (the menu's hit code looks for them).
pub const OPEN_FOLDER: &str = "MusicOpenFolder";
/// See [`OPEN_FOLDER`].
pub const PREVIOUS_TRACK: &str = "MusicPrevious";
/// See [`OPEN_FOLDER`].
pub const NEXT_TRACK: &str = "MusicNext";
/// The names of the Species Names page's two lines of text.
pub const NAMES_COUNT: &str = "NamesCount";
/// See [`NAMES_COUNT`].
pub const NAMES_SAMPLE: &str = "NamesSample";
/// The names of the Species Names page's three buttons.
pub const NAMES_OPEN_FILE: &str = "NamesOpenFile";
/// See [`NAMES_OPEN_FILE`].
pub const NAMES_RELOAD: &str = "NamesReload";
/// See [`NAMES_OPEN_FILE`].
pub const NAMES_NEW_SAMPLES: &str = "NamesNewSamples";
/// The distance from one button of the column to the next in the capture (an 80 high button and a gap of 20).
const BUTTON_STEP: f32 = 100.0;
/// The gap between the two buttons of a row.
const BUTTON_GAP: f32 = 10.0;
/// How many rows the group of the capture that is copied has (the Save Settings group), and so how many places a page's rows have.
pub const SLOTS: usize = 6;

/// One row of a page.
#[derive(Clone, Copy, Debug)]
pub enum Row {
    /// A check box row bound to the setting with this label.
    Toggle(&'static str),
    /// A slider row bound to the setting with this label.
    Slider(&'static str),
    /// A dropdown row bound to the setting with this label.
    Dropdown(&'static str),
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
    /// A label and two buttons side by side (each a node name and what it says).
    Pair {
        /// The row's label.
        label: &'static str,
        /// The left button.
        left: (&'static str, &'static str),
        /// The right button.
        right: (&'static str, &'static str),
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

/// The Music page (the jukebox).
pub const MUSIC: Page = Page {
    session: MUSIC_SESSION,
    title: "Music",
    rows: &[
        Row::Toggle(crate::jukebox::LABEL_PLAY),
        Row::Toggle(crate::jukebox::LABEL_SHUFFLE),
        Row::Slider(crate::jukebox::LABEL_VOLUME),
        Row::Info { label: "Now Playing", node: NOW_PLAYING },
        Row::Button { label: "Music Folder", node: OPEN_FOLDER, text: "Open Folder" },
        Row::Pair { label: "Skip Track", left: (PREVIOUS_TRACK, "Previous"), right: (NEXT_TRACK, "Next") },
    ],
};

/// The Species Names page (the editable name list).
pub const NAMES: Page = Page {
    session: NAMES_SESSION,
    title: "Species Names",
    rows: &[
        Row::Dropdown(crate::name_list::LABEL_MODE),
        Row::Info { label: "My List", node: NAMES_COUNT },
        Row::Button { label: "Names File", node: NAMES_OPEN_FILE, text: "Open File" },
        Row::Button { label: "Reload File", node: NAMES_RELOAD, text: "Reload" },
        Row::Info { label: "Sample Names", node: NAMES_SAMPLE },
        Row::Button { label: "Other Samples", node: NAMES_NEW_SAMPLES, text: "Show New" },
    ],
};

/// The Towers and Streams page (the switches of the gene selector towers, the hydrothermal vents, the force towers and the streams, and where the towers are placed from and
/// kept).
pub const GENE: Page = Page {
    session: GENE_SESSION,
    title: "Towers & Streams",
    rows: &[
        Row::Toggle(LABEL_GENE_TOWERS),
        Row::Toggle(LABEL_VENTS),
        Row::Toggle(LABEL_FORCE),
        Row::Toggle(LABEL_WHIRL),
        Row::Toggle(LABEL_STREAMS),
        Row::Info { label: "Placing", node: GENE_INFO_PLACE },
    ],
};

/// The Extra Hotkeys page: the keys of the hotkeys this edition adds, each with a button (a click, then the new key).
pub const KEYS: Page = Page {
    session: KEYS_SESSION,
    title: "Extra Hotkeys",
    rows: &[
        Row::Button { label: "Hide Objects", node: KEY_HIDE_OBJECTS, text: "V" },
        Row::Button { label: "Default Keys", node: KEYS_RESET, text: "Reset" },
        Row::Info { label: "It Hides", node: KEYS_INFO_HIDES },
        Row::Info { label: "It Keeps", node: KEYS_INFO_STAYS },
        Row::Info { label: "Change Key", node: KEYS_INFO_STATE },
    ],
};

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
pub const PAGES: [&Page; 5] = [&MUSIC, &NAMES, &GENE, &KEYS, &CALIBRATION];

/// The page that is made as the session called `name`, if it is one of the extra pages.
#[must_use]
pub fn page_named(name: &str) -> Option<&'static Page> {
    PAGES.into_iter().find(|page| page.session == name)
}

/// The text help of the Towers page's switches (the title and the text), by label.
#[must_use]
pub fn help_of(label: &str) -> Option<(&'static str, &'static str)> {
    let label = label.trim();
    if label.eq_ignore_ascii_case(LABEL_GENE_TOWERS) {
        Some((
            LABEL_GENE_TOWERS,
            "If enabled, gene selector towers kill bibites by the value of a gene, every cycle (place one from the placer panel at the bottom left). If not, they do nothing. With no tower in the world it changes nothing either way.",
        ))
    } else if label.eq_ignore_ascii_case(LABEL_VENTS) {
        Some((
            LABEL_VENTS,
            "If enabled, hydrothermal vents shoot plant pellets in random directions, at the speed, size and rate set in their panel, up to their cap (place one from the placer panel at the bottom left). If not, they do nothing. With no vent in the world it changes nothing either way.",
        ))
    } else if label.eq_ignore_ascii_case(LABEL_FORCE) {
        Some((
            LABEL_FORCE,
            "If enabled, force towers pull the bibites, pellets and eggs in their range towards them, or push them away, with the strength, range and falloff set in their panel (place one from the placer panel at the bottom left). If not, they do nothing. With no force tower in the world it changes nothing either way.",
        ))
    } else if label.eq_ignore_ascii_case(LABEL_WHIRL) {
        Some((
            LABEL_WHIRL,
            "If enabled, whirlpool towers turn the bibites, pellets and eggs in their range round the tower, clockwise or counterclockwise, with the speed, range and falloff set in their panel (place one from the placer panel at the bottom left). If not, they do nothing. With no whirlpool tower in the world it changes nothing either way.",
        ))
    } else if label.eq_ignore_ascii_case(LABEL_STREAMS) {
        Some((
            LABEL_STREAMS,
            "If enabled, the streams of the Zones tab (a start, a finish, a width and a strength) push the bibites, pellets and eggs inside them from the start towards the finish. If not, they stay where they are drawn and push nothing. With no stream in the world it changes nothing either way.",
        ))
    } else {
        None
    }
}

/// The text of a line of a page that always says the same thing (the node's name tells which).
#[must_use]
pub fn static_line(node: &str) -> Option<&'static str> {
    Some(match node {
        GENE_INFO_PLACE => "Placer panel (bottom left); kept beside the save",
        KEYS_INFO_HIDES => "Bibites, eggs, pellets, towers, particles",
        KEYS_INFO_STAYS => "Panels, graphs, zones, grid; the world runs on",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------------------------------------------------------------------
// The buttons in the column
// ---------------------------------------------------------------------------------------------------------------------------------------------------------

/// Adds the Music and Species Names buttons under the Hotkeys one in the column of `session` (a capture of a Settings page); a session without the column, or one
/// that has the buttons, is left as it is.
pub(crate) fn add_page_buttons(session: &mut RuntimeSession) {
    for canvas in &mut session.canvases {
        if add_buttons_to(canvas) {
            return;
        }
    }
}

fn add_buttons_to(node: &mut Node) -> bool {
    if node.path.ends_with("/SettingsPanel/SelectorPanel/List") {
        add_button_after(node, "Controls Button", MUSIC_BUTTON, MUSIC_BUTTON_LABEL, 1.0);
        add_button_after(node, MUSIC_BUTTON, NAMES_BUTTON, NAMES_BUTTON_LABEL, 2.0);
        add_button_after(node, NAMES_BUTTON, GENE_BUTTON, GENE_BUTTON_LABEL, 3.0);
        add_button_after(node, GENE_BUTTON, KEYS_BUTTON, KEYS_BUTTON_LABEL, 4.0);
        add_button_after(node, KEYS_BUTTON, CALIBRATION_BUTTON, CALIBRATION_BUTTON_LABEL, 5.0);
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
    slider: Node,
    dropdown: Node,
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
    // The Game page's own Save Settings group has a dropdown row; a capture without one gives a check box row's copy (the page still builds).
    let dropdown = first("SettingDropdown").unwrap_or_else(|| toggle.clone());
    // Where a row's parts sit in the slider row: the name takes the left, the control (slider, here the buttons or the text) the right.
    let parts = Parts {
        name_end: child(&slider, "Name")?.native_rect?[2],
        control_start: child(&slider, "NormalSecton")?.native_rect?[0],
        value_start: child(&slider, "FloatInfoText")?.native_rect?[0],
        toggle,
        slider,
        dropdown,
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
        Row::Slider(label) => row_at(&parts.slider, slot, label),
        Row::Dropdown(label) => row_at(&parts.dropdown, slot, label),
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
        Row::Pair { label, left, right } => {
            let mut row = plain_row(parts, slot, label);
            let [_, y0, edge, y1] = row.native_rect?;
            let half = (edge - parts.control_start - BUTTON_GAP) / 2.0;
            let first = button_from(button, &row.path, left.0, left.1, [parts.control_start, y0, parts.control_start + half, y1]);
            let second = button_from(button, &row.path, right.0, right.1, [edge - half, y0, edge, y1]);
            row.children.push(first);
            row.children.push(second);
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
    fn the_two_buttons_are_the_hotkeys_buttons_copies_one_and_two_steps_lower_with_their_own_paths_and_labels() {
        let mut session = session();
        add_page_buttons(&mut session);
        let list = list(&session);
        let names: Vec<&str> = list.children.iter().map(|button| button.name.as_str()).collect();
        assert_eq!(names, ["GraphicsButton", "Gameplay Button", "Controls Button", MUSIC_BUTTON, NAMES_BUTTON, GENE_BUTTON, KEYS_BUTTON, CALIBRATION_BUTTON, "spacer", "Reset Warnings"]);
        assert_eq!(list.children.iter().map(|button| button.sibling_index).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9], "the ones after them moved down");
        let hotkeys = list.children[2].native_rect.unwrap();
        for (at, steps, label) in [(3, 1.0, MUSIC_BUTTON_LABEL), (4, 2.0, NAMES_BUTTON_LABEL), (5, 3.0, GENE_BUTTON_LABEL), (6, 4.0, KEYS_BUTTON_LABEL), (7, 5.0, CALIBRATION_BUTTON_LABEL)] {
            let button = &list.children[at];
            let rect = button.native_rect.unwrap();
            assert!((hotkeys[1] - rect[1] - BUTTON_STEP * steps).abs() < 1e-3 && (hotkeys[3] - rect[3] - BUTTON_STEP * steps).abs() < 1e-3, "{label}");
            assert!(button.path.ends_with(&format!("/List/{}", button.name)) && button.children[0].path.ends_with(&format!("/List/{}/Text (TMP)", button.name)));
            assert_eq!(button.children[0].texts[0].text, label);
        }
        assert_eq!(list.children[2].children[0].texts[0].text, "Hotkeys", "the copies left the original alone");
        // Added once only; a session without the column is left as it is.
        let before = format!("{:?}", list.children);
        add_page_buttons(&mut session);
        assert_eq!(format!("{:?}", self::list(&session).children), before);
        let mut other = RuntimeSession { canvases: vec![node("UI", "UI", None)], ..RuntimeSession::default() };
        add_page_buttons(&mut other);
        assert!(other.canvases[0].children.is_empty());
    }

    #[test]
    fn the_music_page_is_one_group_of_six_rows_in_the_places_of_the_captures_rows() {
        let mut session = session();
        assert!(make_page(&mut session, &MUSIC));
        let holder = &session.canvases[0].children[1];
        assert_eq!(holder.children.len(), 1, "the other groups of the Game page are gone");
        assert_eq!(holder.children[0].children[0].texts[0].text, "Music");
        let rows = rows(&session);
        assert_eq!(rows.len(), SLOTS);
        let labels: Vec<&str> = rows.iter().map(label_of).collect();
        assert_eq!(labels, ["Play Music", "Shuffle Music", "Music Volume", "Now Playing", "Music Folder", "Skip Track"]);
        // Each row is where the capture's row of the same place was.
        for (slot, row) in rows.iter().enumerate() {
            let y0 = 855.0 - slot as f32 * 60.0;
            assert_eq!(row.native_rect.unwrap(), [115.0, y0, 1222.0, y0 + 40.0], "row {slot}");
            assert_eq!(row.sibling_index, slot as i32);
        }
        // The first two are check box rows and the third a slider row: the menu binds them to their settings by those names.
        assert!(rows[0].name.starts_with("SettingToggle") && rows[1].name.starts_with("SettingToggle") && rows[2].name.starts_with("SettingSlider"));
        assert!(rows[0].children.iter().any(|part| part.name == "Toggle") && rows[2].children.iter().any(|part| part.name == "NormalSecton"));
    }

    #[test]
    fn the_playing_row_has_a_live_text_and_the_folder_and_track_rows_their_buttons_in_the_control_column() {
        let mut session = session();
        assert!(make_page(&mut session, &MUSIC));
        let rows = rows(&session);
        // Now Playing: no check box, the text from where the slider rows' values start to the row's end.
        let playing = &rows[3];
        assert!(playing.children.iter().all(|part| part.name != "Toggle"));
        let text = playing.children.iter().find(|part| part.name == NOW_PLAYING).expect("the live text");
        let y0 = 855.0 - 3.0 * 60.0;
        assert_eq!(text.native_rect.unwrap(), [563.0, y0, 1222.0, y0 + 40.0]);
        assert!(text.path.ends_with("/SettingToggle(Clone)/NowPlaying"));
        let name = playing.children.iter().find(|part| part.name == "Name").unwrap();
        assert_eq!(name.native_rect.unwrap()[2], 548.0, "the name stops where the slider rows' names stop");
        // Music Folder: one button over the control column.
        let open = rows[4].children.iter().find(|part| part.name == OPEN_FOLDER).expect("the folder button");
        let y0 = 855.0 - 4.0 * 60.0;
        assert_eq!(open.native_rect.unwrap(), [813.0, y0, 1222.0, y0 + 40.0]);
        assert_eq!(open.children[0].texts[0].text, "Open Folder");
        assert_eq!(open.children[0].native_rect, open.native_rect, "the text fills the button");
        assert!(open.path.ends_with("/SettingToggle(Clone)/MusicOpenFolder") && open.children[0].path.ends_with("/MusicOpenFolder/Text (TMP)"));
        // Skip Track: two buttons of one width with the gap between.
        let previous = rows[5].children.iter().find(|part| part.name == PREVIOUS_TRACK).expect("previous");
        let next = rows[5].children.iter().find(|part| part.name == NEXT_TRACK).expect("next");
        let (p, n) = (previous.native_rect.unwrap(), next.native_rect.unwrap());
        assert_eq!((p[0], n[2]), (813.0, 1222.0));
        assert!((p[2] - p[0] - (n[2] - n[0])).abs() < 1e-3 && (n[0] - p[2] - BUTTON_GAP).abs() < 1e-3, "{p:?} {n:?}");
        assert_eq!((previous.children[0].texts[0].text.as_str(), next.children[0].texts[0].text.as_str()), ("Previous", "Next"));
    }

    #[test]
    fn the_species_names_page_has_a_dropdown_two_lines_of_text_and_three_buttons() {
        let mut session = session();
        assert!(make_page(&mut session, &NAMES));
        assert_eq!(session.canvases[0].children[1].children[0].children[0].texts[0].text, "Species Names");
        let rows = rows(&session);
        let labels: Vec<&str> = rows.iter().map(label_of).collect();
        assert_eq!(labels, ["Species Names", "My List", "Names File", "Reload File", "Sample Names", "Other Samples"]);
        // The first row is the capture's dropdown row (the menu binds it to its setting by its name and opens its list from its `Dropdown`).
        assert!(rows[0].name.starts_with("SettingDropdown") && rows[0].children.iter().any(|part| part.name == "Dropdown"));
        for (slot, row) in rows.iter().enumerate() {
            let y0 = 855.0 - slot as f32 * 60.0;
            assert_eq!(row.native_rect.unwrap(), [115.0, y0, 1222.0, y0 + 40.0], "row {slot}");
        }
        let has = |row: usize, node: &str| rows[row].children.iter().any(|part| part.name == node);
        assert!(has(1, NAMES_COUNT) && has(4, NAMES_SAMPLE));
        assert!(has(2, NAMES_OPEN_FILE) && has(3, NAMES_RELOAD) && has(5, NAMES_NEW_SAMPLES));
        let reload = rows[3].children.iter().find(|part| part.name == NAMES_RELOAD).unwrap();
        assert_eq!((reload.children[0].texts[0].text.as_str(), reload.native_rect.unwrap()[0]), ("Reload", 813.0));
    }

    #[test]
    fn the_towers_and_streams_page_has_the_five_switches_and_a_line_that_always_says_the_same() {
        let mut session = session();
        assert!(make_page(&mut session, &GENE));
        let rows = rows(&session);
        assert_eq!(rows.len(), 6, "six rows: the five switches and the line");
        let labels: Vec<&str> = rows.iter().map(label_of).collect();
        assert_eq!(labels, ["Gene Selector Towers", "Hydrothermal Vents", "Force Towers", "Whirlpool Towers", "Streams", "Placing"]);
        for row in &rows[..5] {
            assert!(row.name.starts_with("SettingToggle") && row.children.iter().any(|part| part.name == "Toggle"));
        }
        assert!(help_of(LABEL_STREAMS).is_some_and(|(title, text)| title == "Streams" && text.contains("start")));
        assert!(help_of(LABEL_WHIRL).is_some_and(|(title, text)| title == "Whirlpool Towers" && text.contains("clockwise")));
        assert!(rows[5].children.iter().any(|part| part.name == GENE_INFO_PLACE), "{GENE_INFO_PLACE}");
        assert!(static_line(GENE_INFO_PLACE).is_some_and(|line| !line.is_empty() && line.chars().count() <= 56), "{GENE_INFO_PLACE}");
        assert!(static_line("NowPlaying").is_none());
    }

    #[test]
    fn the_calibration_page_is_a_check_box_a_button_and_four_lines_in_the_places_of_the_captures_rows() {
        let mut session = session();
        assert!(make_page(&mut session, &CALIBRATION));
        assert_eq!(session.canvases[0].children[1].children[0].children[0].texts[0].text, "Calibration");
        let rows = rows(&session);
        assert_eq!(rows.len(), SLOTS);
        let labels: Vec<&str> = rows.iter().map(label_of).collect();
        assert_eq!(labels, ["Use Calibration", "Calibration Test", "Status", "Machine", "Multi-Core", "Graphics"]);
        // The check box row is the capture's (the menu binds it to its setting by its name); the test button sits over the control column.
        assert!(rows[0].name.starts_with("SettingToggle") && rows[0].children.iter().any(|part| part.name == "Toggle"));
        let run = rows[1].children.iter().find(|part| part.name == names::RUN).expect("the button");
        let y0 = 855.0 - 60.0;
        assert_eq!(run.native_rect.unwrap(), [813.0, y0, 1222.0, y0 + 40.0]);
        assert_eq!(run.children[0].texts[0].text, "Run Test");
        assert!(run.path.ends_with("/SettingToggle(Clone)/CalibrationRun") && run.children[0].path.ends_with("/CalibrationRun/Text (TMP)"));
        for (row, node) in [(2, names::STATUS), (3, names::MACHINE), (4, names::MULTICORE), (5, names::GRAPHICS)] {
            assert!(rows[row].children.iter().all(|part| part.name != "Toggle"), "no box in a line of text");
            let text = rows[row].children.iter().find(|part| part.name == node).expect("the live text");
            let y0 = 855.0 - row as f32 * 60.0;
            assert_eq!(text.native_rect.unwrap(), [563.0, y0, 1222.0, y0 + 40.0], "{node}");
        }
    }

    #[test]
    fn the_pages_are_found_by_their_session_names() {
        assert_eq!(page_named(MUSIC_SESSION).map(|page| page.title), Some("Music"));
        assert_eq!(page_named(NAMES_SESSION).map(|page| page.title), Some("Species Names"));
        assert_eq!(page_named(GENE_SESSION).map(|page| page.title), Some("Towers & Streams"));
        assert_eq!(page_named(KEYS_SESSION).map(|page| page.title), Some("Extra Hotkeys"));
        assert_eq!(page_named(CALIBRATION_SESSION).map(|page| page.title), Some("Calibration"));
        assert!(page_named("main_menu_settings_gameplay").is_none());
    }

    #[test]
    fn a_capture_without_the_parts_gives_no_page_and_no_change() {
        let mut empty = RuntimeSession { canvases: vec![node("UI", "UI", None)], ..RuntimeSession::default() };
        assert!(!make_page(&mut empty, &MUSIC));
        let mut no_game_page = session();
        no_game_page.canvases[0].children.pop();
        assert!(!make_page(&mut no_game_page, &NAMES));
    }
}
