//! The sizes of the Info and Genes panels, laid out the way the game lays them out.
//!
//! The species panels hang in `SpeciesPanels`, a `HorizontalLayoutGroup` with a `ContentSizeFitter`: the shown panel is as wide as the widest
//! thing in it asks (Unity's preferred width of every row), so a species with a long name, or a long parent's, has a wider panel than one with
//! a short name, and the tab row (`Species Tabs`, stretched over the panel) keeps the close tab at the panel's right end. A live capture of
//! the game (hidden window, `GET /d/ui` after `POST /d/open_species`) gives the rule:
//!
//! * Info panel = 20 + the Preview Section + 10 (`row spacer`) + 35 (the buttons). The Preview Section is as wide as its widest row: the name
//!   row (the name, a 40 wide spacer, the 22 rename icon, less the row's -5 padding), the "Species ID:" row, the 200 preview, or the root /
//!   parent row (125 for the titles, 20 padding and the longer of the two names in the buttons). The layout is solved from the prefab
//!   (`species_info_layout_data.rs`, [`crate::unity_layout`]) with the species' texts.
//! * Genes panel = 15 + its widest row: a gene line (the longest gene name in LiberationSans 14, 15 of right margin, the 90 value and the 100
//!   difference), the name (Roboto-Bold 20 and 20 of margin), the parent row (150 for the title, the button, the 80 distance, 5) or the
//!   compared row (the same with the dropdown, whose label has 30 of padding).

use crate::species_info_layout_data::SPECIES_INFO_PANEL;
use crate::unity_layout::{self, Elem, LNode, Measure, Request, Solved};

use super::{InfoLayout, OVERLAY};

/// The panel's root node.
const ROOT: &str = "Species Info Panel";
/// `Preview Section`.
const PREVIEW: &str = "Top Section/Preview Section";
/// `Roboto-Bold SDF NoOutline`: the names and the numbers.
const BOLD: &str = "Roboto-Bold SDF NoOutline";
/// `LiberationSans SDF`: the gene names.
const LIBERATION: &str = "LiberationSans SDF";

/// What the Info panel's layout depends on (`SpeciesInfoPanel.FillPanel`, `EditSpeciesName`).
pub(crate) struct InfoTexts<'a> {
    /// The species' name.
    pub name: &'a str,
    /// The species' ID.
    pub id: String,
    /// The root species' name.
    pub root: &'a str,
    /// The parent species' name (`None` shows "None" in place of the button).
    pub parent: Option<&'a str>,
    /// Time of creation, generation, alive members, alive descendants.
    pub values: [String; 4],
    /// The name is being edited: whether the format warning shows.
    pub editing: Option<bool>,
}

/// The panel solved for `editing` (the editor as wide as the name was at its start: `nameEditorLayout.minWidth`).
fn solve_info(texts: &InfoTexts<'_>, editing: Option<bool>, editor_width: f32, measure: &dyn Measure) -> Solved {
    let active = |path: &str, node: &LNode| {
        if path.ends_with("NameRow/InputField (TMP)") {
            editing.is_some()
        } else if path.ends_with("NameRow/Name Row") {
            editing.is_none()
        } else if path.ends_with("NameRow/WarningIcon") {
            editing == Some(true)
        } else if path.ends_with("Buttons/NoneText") {
            texts.parent.is_none()
        } else if path.ends_with("Buttons/ParentButton") {
            texts.parent.is_some()
        } else {
            node.active
        }
    };
    let text = |path: &str, _: &LNode| -> Option<String> {
        if path.ends_with("/Species Name") {
            Some(texts.name.to_owned())
        } else if path.ends_with("IDRow/IDText") {
            Some(texts.id.clone())
        } else if path.ends_with("Rootbutton/root Name") {
            Some(texts.root.to_owned())
        } else if path.ends_with("ParentButton/parent Name") {
            texts.parent.map(str::to_owned)
        } else if path.ends_with("Line/timeOfCreation") {
            Some(texts.values[0].clone())
        } else if path.ends_with("Line (1)/FloatInfoText") {
            Some(texts.values[1].clone())
        } else if path.ends_with("Line (3)/memeber") {
            Some(texts.values[2].clone())
        } else if path.ends_with("Line (2)/descendants") {
            Some(texts.values[3].clone())
        } else {
            None
        }
    };
    // Two texts have a right margin of 10, which TMP adds to their preferred width; the editor's minimum is set by the code.
    let elem = |path: &str, _: &LNode| -> Option<Elem> {
        let preferred = |text: &str| Elem {
            ignore: false,
            min: [-1.0; 2],
            pref: [measure.text_width("Roboto-Regular SDF", text, 14.0, true) + 10.0, -1.0],
            flex: [-1.0; 2],
        };
        if path.ends_with("NameRow/InputField (TMP)") {
            Some(Elem {
                ignore: false,
                min: [editor_width, -1.0],
                pref: [-1.0; 2],
                flex: [1.0, -1.0],
            })
        } else if path.ends_with("IDRow/Title") {
            Some(preferred("Species ID:"))
        } else if path.ends_with("IDRow/IDText") {
            Some(preferred(&texts.id))
        } else {
            None
        }
    };
    unity_layout::solve(
        &SPECIES_INFO_PANEL,
        &Request {
            active: &active,
            text: &text,
            elem: Some(&elem),
        },
        measure,
    )
}

/// The Info panel's rectangles for a species (reference px, the panel hanging from the overlay's corner).
pub(crate) fn info_layout(texts: &InfoTexts<'_>, measure: &dyn Measure) -> InfoLayout {
    let name_group = format!("{ROOT}/{PREVIEW}/SpeciesNameAndID/NameRow/Name Row");
    let resting = solve_info(texts, None, 0.0, measure);
    let solved = match texts.editing {
        None => resting,
        Some(_) => {
            let width = resting.rects.get(&name_group).map_or(0.0, |rect| rect[2]);
            solve_info(texts, texts.editing, width, measure)
        }
    };
    let at = |rel: &str| -> [f32; 4] {
        solved
            .rects
            .get(&format!("{ROOT}/{rel}"))
            .map_or([0.0; 4], |r| [OVERLAY.x + r[0], OVERLAY.y + r[1], r[2], r[3]])
    };
    let name_rows = format!("{PREVIEW}/SpeciesNameAndID/NameRow");
    let id_row = format!("{PREVIEW}/SpeciesNameAndID/IDRow");
    let references = format!("{PREVIEW}/SpeciesReferences");
    let holder = |name: &str| at(&format!("Top Section/Buttons Holder/{name}"));
    let body = |name: &str| at(&format!("Body/TextBody/{name}"));
    let panel = solved
        .rects
        .get(ROOT)
        .map_or([OVERLAY.x, OVERLAY.y, 0.0, 0.0], |r| {
            [OVERLAY.x + r[0], OVERLAY.y + r[1], r[2], r[3]]
        });
    InfoLayout {
        panel,
        name_row: if texts.editing.is_some() {
            at(&name_rows)
        } else {
            at(&format!("{name_rows}/Name Row"))
        },
        name_field: at(&format!("{name_rows}/InputField (TMP)")),
        rename_icon: at(&format!("{name_rows}/Name Row/Name/Icon")),
        warning_icon: at(&format!("{name_rows}/WarningIcon")),
        id_row: at(&id_row),
        id_value: at(&format!("{id_row}/IDText")),
        preview: at(&format!("{PREVIEW}/Preview Item")),
        root_title: at(&format!("{references}/titles/RootTitle")),
        parent_title: at(&format!("{references}/titles/ParentTitle")),
        root_button: at(&format!("{references}/Buttons/Rootbutton")),
        parent_button: if texts.parent.is_some() {
            at(&format!("{references}/Buttons/ParentButton"))
        } else {
            at(&format!("{references}/Buttons/NoneText"))
        },
        buttons: [
            holder("Favorite Toggle"),
            holder("SaveSpeciesButton"),
            holder("PlaceSpeciesButton"),
            holder("SelectButton"),
            holder("PruneButton"),
            holder("Edit"),
        ],
        lines: [body("Line"), body("Line (1)"), body("Line (3)"), body("Line (2)")],
        notes_label: body("Notes Section/Text (TMP)"),
        notes: body("Notes Section/Description"),
    }
}

/// What the Genes panel's width depends on.
pub(crate) struct GenesTexts<'a> {
    /// The species' name (20 bold).
    pub name: &'a str,
    /// The parent's name, `None` for a root species ("None" in the row, no distance).
    pub parent: Option<&'a str>,
    /// The distance to the parent (16 bold).
    pub parent_distance: &'a str,
    /// The compared species' name (the dropdown's label).
    pub compared: &'a str,
    /// The distance to it.
    pub compared_distance: &'a str,
}

/// The inner width of the Genes panel (the panel less its padding of 10 and 5): the widest of its rows.
pub(crate) fn genes_inner_width(texts: &GenesTexts<'_>, measure: &dyn Measure) -> f32 {
    let bold = |text: &str, size: f32| measure.text_width(BOLD, text, size, false);
    // A gene line: the name (TMP adds its 15 of right margin), the 90 value and the 100 difference.
    let longest = crate::gene_info_data::GENES
        .iter()
        .map(|gene| measure.text_width(LIBERATION, &format!("{}: ", gene.name), 14.0, false))
        .fold(0.0_f32, f32::max);
    let lines = longest + 15.0 + 90.0 + 100.0;
    let name = bold(texts.name, 20.0) + 20.0;
    // The distances have a minimum width of 80 and a left margin of 5.
    let distance = |text: &str| (bold(text, 16.0) + 5.0).max(80.0);
    let parent = 150.0
        + texts.parent.map_or_else(
            || bold("None", 14.0),
            |parent| 20.0 + bold(parent, 14.0) + distance(texts.parent_distance),
        )
        + 5.0;
    let compared = 150.0 + (30.0 + bold(texts.compared, 14.0)) + distance(texts.compared_distance) + 5.0;
    lines.max(name).max(parent).max(compared)
}
