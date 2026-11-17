//! The Bibite Editor's screen (`BibiteEditorPanel`, hand drawn from its prefab and the reference screenshot): the title, the buttons along
//! the top, the Genes tab (35 `FloatSettingSlider` rows: name, value, slider, reset), the Brain tab's place, and on the right the preview
//! frame, the name field with its three warning icons and the description field.
//!
//! The geometry is the 1920 x 1080 reference's: the prefab's layout groups give the structure (a title, a row of buttons, a holder with
//! the tab panels left and the previewer right), the screenshot the sizes. Rows are 60 apart and the list scrolls a row at a time.

use bibites_core::genome::{GENE_DEFS, N_GENES};
use bibites_core::math::Vec2;

use crate::app::App;
use crate::brain_editor::BrainEdit;
use crate::brain_editor_ui::{BrainAction, BrainView};
use crate::gene_analysis::{gene_format, gene_info};
use crate::species_panel::Drawer;
use crate::template_editor::TemplateEditor;
use crate::tooltip::{Tip, TipRect};
use crate::ui::{Rgba, UiAction, UiAssets, UiDraw, UiFrame, UiId};

/// The two tabs (`genesTab`, `brainTab`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    /// The genes' sliders.
    Genes,
    /// The brain's nodes and synapses.
    Brain,
}

/// The two text fields (`speciesNameHandle`, `speciesDescHandle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextField {
    /// The template's name.
    Name,
    /// The template's description.
    Description,
}

/// What a control of the editor does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditorAction {
    /// The Genes / Brain buttons (`BibiteBrainEditor.ClosePanel` / `OpenPanel`).
    Tab(Tab),
    /// "Save and continue" (`SaveAndContinue`): writes the template and stays.
    Save,
    /// "Save and submit" (`SaveAndSubmit`): writes the template, hands it to the caller and closes.
    Submit,
    /// The close button (`BibiteEditorPanel.ClosePanel`).
    Back,
    /// A gene slider pressed (the gene's index, the track's ends in reference px).
    GeneSlide(u8, u16, u16),
    /// A gene's reset button (`ResetValue`).
    GeneReset(u8),
    /// A text field pressed: it takes the focus.
    Focus(TextField),
    /// The list's scroll bar track pressed: the list pages towards the click (`true` up, `false` down).
    Page(bool),
    /// The preview's info button (`BibiteEditorPreview.ToggleOpenPreview`): the stats panel opens and closes.
    Info,
    /// The sprite button (`SaveSpriteOfBibite`): writes the preview's bibite as a picture.
    SaveSprite,
    /// A control of the Brain tab.
    Brain(BrainAction),
}

/// The editor's screen state (what is shown and held, not what is edited).
#[derive(Clone, Debug)]
pub struct EditorView {
    /// The tab that shows.
    pub tab: Tab,
    /// How many rows the genes list is scrolled.
    pub scroll_rows: usize,
    /// The text field that has the focus.
    pub focus: Option<TextField>,
    /// A gene slider is held: the gene and the track's ends.
    pub drag: Option<(u8, u16, u16)>,
    /// The stats panel over the name and the description is open (`Bibite Stats Preview`).
    pub info_open: bool,
    /// Seconds since the last key (the caret blinks from there).
    pub idle: f32,
    /// What the last save said.
    pub status: String,
    /// The Brain tab's canvas and panels.
    pub brain: BrainView,
}

impl Default for EditorView {
    fn default() -> Self {
        Self {
            tab: Tab::Genes,
            scroll_rows: 0,
            focus: None,
            drag: None,
            info_open: false,
            idle: 0.0,
            status: String::new(),
            brain: BrainView::default(),
        }
    }
}

/// An open editor: what is edited and how it shows.
#[derive(Clone, Debug)]
pub struct EditorSession {
    /// The template being edited.
    pub model: TemplateEditor,
    /// The Brain tab's editor (`BibiteBrainEditor`), filled with the template's brain and the positions it keeps.
    pub brain: BrainEdit,
    /// The screen's state.
    pub view: EditorView,
    /// The editor has "Save and submit" (`saveAndSubmitButton.SetActive(actionOnSubmit != null)`): the Library's Edit Bibite and
    /// the inspector's Edit give it an action, "Create New" does not.
    pub submit: bool,
}

impl EditorSession {
    /// A session on `model`, at the Genes tab, with "Save and submit", for a window of `screen` pixels (the brain editor sizes its
    /// canvas by the window: `FillEditor`).
    #[must_use]
    pub fn for_screen(model: TemplateEditor, screen: [f32; 2]) -> Self {
        let anchors = model
            .anchors
            .iter()
            .map(|(innovation, anchor)| (*innovation, (anchor.x, anchor.y)))
            .collect();
        let brain = BrainEdit::fill_for_screen(
            model.brain.to_node_records(),
            &model.brain.to_synapse_records(),
            &anchors,
            screen,
        );
        Self {
            model,
            brain,
            view: EditorView::default(),
            submit: true,
        }
    }

    /// Whether Save and Save & Submit work (`UpdateValidity`: `nameValid && brainEditor.valid`).
    #[must_use]
    pub fn can_save(&self) -> bool {
        self.model.can_save() && self.brain.is_valid()
    }
}

// ---------------------------------------------------------------------------------------- the geometry

const BACKGROUND: Rgba = [0, 0, 31, 255];
const WHITE: Rgba = [255, 255, 255, 255];
const INK: Rgba = [38, 38, 38, 255];
/// The left panel (`GenesTab`, `BrainTab`) and the area its rows show in.
pub(crate) const LIST: [f32; 4] = [12.0, 78.0, 1328.0, 990.0];
/// A row's height (the `VerticalLayoutGroup`'s row and its spacing) and where the first one starts.
const ROW_H: f32 = 60.0;
const FIRST_ROW: f32 = 96.0;
/// The rows that show at once.
pub const VISIBLE_ROWS: usize = 16;
/// The name's left edge, the value text's middle, the slider's track ends and the reset button.
const NAME_X: f32 = 48.0;
const VALUE_X: f32 = 700.0;
const TRACK: [f32; 2] = [856.0, 1236.0];
const RESET_X: f32 = 1258.0;
/// The scroll bar's track.
const SCROLL: [f32; 4] = [1312.0, 94.0, 14.0, 958.0];
/// The right column: the preview's frame, the body's frame, the name row, the description.
const PREVIEW_FRAME: [f32; 4] = [1360.0, 78.0, 548.0, 392.0];
const BODY_FRAME: [f32; 4] = [1360.0, 478.0, 548.0, 590.0];
const NAME_FIELD: [f32; 4] = [1476.0, 508.0, 281.0, 39.0];
const DESC_FIELD: [f32; 4] = [1388.0, 596.0, 492.0, 440.0];
/// The info button on the preview's bottom right.
const INFO_BUTTON: [f32; 4] = [1846.0, 410.0, 50.0, 50.0];
/// The three warning icons' middles and their size.
const WARNING_X: [f32; 3] = [1780.0, 1820.0, 1860.0];
const WARNING_Y: f32 = 527.0;
const WARNING_SIZE: f32 = 35.0;

/// The top buttons (62 square, y 14): Genes, Brain, then (right) Save sprite, Save and continue, Submit, Back, and the tooltips'
/// paths of each (`Buttons` is a `HorizontalLayoutGroup` of 70 pt buttons with flexible spacers).
const BUTTON_Y: f32 = 14.0;
const BUTTON_SIZE: f32 = 62.0;
const TIP_BASE: &str = "UICanvas/MainUI/Layer2/BibiteSelector/BibiteEditor/Panel/Buttons/";
const BUTTONS: [(f32, &str, EditorAction, &str); 6] = [
    (
        28.0,
        "dna_icon",
        EditorAction::Tab(Tab::Genes),
        "GenesButton",
    ),
    (
        108.0,
        "brain_icon",
        EditorAction::Tab(Tab::Brain),
        "brainButton",
    ),
    (
        1490.0,
        "bibite_import_tool",
        EditorAction::SaveSprite,
        "SaveSprite",
    ),
    (1630.0, "save_icon", EditorAction::Save, "SaveAndContinue"),
    (1712.0, "checkmark_icon", EditorAction::Submit, "Submit"),
    (1826.0, "exit_icon", EditorAction::Back, "BackButton"),
];
/// Without "Save and submit" (`SetActive(false)`) the layout group packs the buttons that were left of it one button and one
/// spacing (80) to the right.
const NO_SUBMIT_SHIFT: f32 = 80.0;

/// Hit ids of the editor.
const HIT_BASE: UiId = 47_000;
const HIT_BUTTON: UiId = HIT_BASE + 1;
const HIT_NAME: UiId = HIT_BASE + 20;
const HIT_DESC: UiId = HIT_BASE + 21;
const HIT_SCROLL: UiId = HIT_BASE + 22;
const HIT_INFO: UiId = HIT_BASE + 24;
const HIT_ROWS: UiId = HIT_BASE + 100;

/// The genes in the order of the list (`BibiteEditorSettings.geneSettings`), as indices of the core's table.
#[must_use]
pub fn row_genes() -> &'static [usize; N_GENES] {
    use std::sync::OnceLock;
    static ROWS: OnceLock<[usize; N_GENES]> = OnceLock::new();
    ROWS.get_or_init(|| {
        let mut rows = [0_usize; N_GENES];
        for (row, info) in crate::gene_info_data::GENES.iter().enumerate() {
            rows[row] = GENE_DEFS
                .iter()
                .position(|def| def.name == info.gene)
                .expect("every setting is a gene");
        }
        rows
    })
}

/// The most the list scrolls.
#[must_use]
pub fn max_scroll_rows() -> usize {
    N_GENES - VISIBLE_ROWS
}

/// The slider's value at `cursor_x` (reference px) on the track `x0..x1` of gene `gene` (`Slider.UpdateDrag`, whole range, linear).
#[must_use]
pub fn value_at(gene: usize, x0: f32, x1: f32, cursor_x: f32) -> f32 {
    let def = &GENE_DEFS[gene];
    if x1 <= x0 {
        return def.default;
    }
    let along = ((cursor_x - x0) / (x1 - x0)).clamp(0.0, 1.0);
    def.min + (def.max - def.min) * along
}

/// Where the handle of gene `gene` at `value` stands on the track (reference px, the handle's middle).
#[must_use]
pub fn handle_x(gene: usize, value: f32) -> f32 {
    let def = &GENE_DEFS[gene];
    let along = ((value - def.min) / (def.max - def.min)).clamp(0.0, 1.0);
    TRACK[0] + (TRACK[1] - TRACK[0]) * along
}

/// A text field after a typed character (`TMP_InputField`: control characters are dropped, a single line field drops new lines).
pub fn type_into(text: &mut String, field: TextField, character: char) {
    if character == '\n' || character == '\r' {
        if field == TextField::Description {
            text.push('\n');
        }
        return;
    }
    if !character.is_control() {
        text.push(character);
    }
}

/// The index of the name check's warning icons that show: `incorrectWarning`, `existingOfficial`, `existingWarning`.
#[must_use]
pub fn warnings(model: &TemplateEditor) -> [bool; 3] {
    [
        model.check.bad_character,
        model.check.official,
        model.check.exists,
    ]
}

/// The tooltip paths of those three icons.
const WARNING_TIPS: [&str; 3] = [
    "UICanvas/MainUI/Layer2/BibiteSelector/BibiteEditor/Panel/Holder/Previewer/BibitePreview/Body/line/incorrectWarning",
    "UICanvas/MainUI/Layer2/BibiteSelector/BibiteEditor/Panel/Holder/Previewer/BibitePreview/Body/line/existingOfficial",
    "UICanvas/MainUI/Layer2/BibiteSelector/BibiteEditor/Panel/Holder/Previewer/BibitePreview/Body/line/existingWarning",
];

/// Draws the editor over the whole screen.
pub fn draw(
    frame: &mut UiFrame,
    app: &App,
    ui: &UiAssets,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    width: f32,
    height: f32,
) {
    let Some(session) = app.bibite_editor.as_ref() else {
        return;
    };
    // The same scale the pointer paths convert with (`UiFrame::scaler`), so a slider drag lands where the slider is drawn.
    let s = UiFrame::scaler(width, height);
    let d = Drawer::new(s, ui, metrics, app.cursor);
    // The panel blocks the world; its background is the prefab's dark blue.
    frame.hit(
        HIT_BASE,
        Vec2::ZERO,
        Vec2::new(width, height),
        UiAction::Consume,
    );
    frame.rect(Vec2::ZERO, Vec2::new(width, height), BACKGROUND);
    // "Bibite Editor": 70 pt, centred.
    let title_px = 70.0 * s;
    let title_width = d.measure("Bibite Editor", title_px, true);
    frame.bold(
        "Bibite Editor",
        Vec2::new(960.0 * s - title_width / 2.0, 6.0 * s),
        title_px,
        WHITE,
    );
    buttons(frame, &d, session);
    match session.view.tab {
        Tab::Genes => genes_tab(frame, &d, app, session),
        Tab::Brain => crate::brain_editor_ui::draw(frame, &d, app, session),
    }
    right_column(frame, &d, app, session);
}

/// The buttons along the top.
fn buttons(frame: &mut UiFrame, d: &Drawer<'_>, session: &EditorSession) {
    for (index, (x, sprite, action, tip)) in BUTTONS.iter().enumerate() {
        if matches!(action, EditorAction::Submit) && !session.submit {
            continue;
        }
        let shifted =
            !session.submit && matches!(action, EditorAction::SaveSprite | EditorAction::Save);
        let x = if shifted { *x + NO_SUBMIT_SHIFT } else { *x };
        let rect = [x, BUTTON_Y, BUTTON_SIZE, BUTTON_SIZE];
        // Save and Save and submit are greyed while the name (or the brain) is not valid (`UpdateValidity`).
        let enabled =
            !matches!(action, EditorAction::Save | EditorAction::Submit) || session.can_save();
        let tint: Rgba = if enabled { WHITE } else { [140, 140, 140, 255] };
        let selected = matches!(action, EditorAction::Tab(tab) if *tab == session.view.tab);
        d.slice(
            frame,
            "base_icon",
            5.0,
            1.0 / 0.5,
            rect,
            if selected { [200, 200, 200, 255] } else { tint },
        );
        d.icon(frame, sprite, rect, tint);
        if enabled {
            frame.hit(
                HIT_BUTTON + index as UiId,
                d.at(rect[0], rect[1]),
                d.at(rect[2], rect[3]),
                UiAction::Editor(*action),
            );
        }
        if let Some(tip) = crate::tooltip::find(&format!("{TIP_BASE}{tip}")) {
            frame.tips.push(TipRect {
                pos: d.at(rect[0], rect[1]),
                size: d.at(rect[2], rect[3]),
                tip,
            });
        }
    }
}

/// The Genes tab: the panel and the visible rows, the scroll bar.
fn genes_tab(frame: &mut UiFrame, d: &Drawer<'_>, app: &App, session: &EditorSession) {
    let view = &session.view;
    d.slice(frame, "gray_panel", 10.0, 1.0 / 0.5, LIST, WHITE);
    let rows = row_genes();
    for slot in 0..VISIBLE_ROWS {
        let row = view.scroll_rows + slot;
        if row >= N_GENES {
            break;
        }
        gene_row(
            frame,
            d,
            app,
            session,
            rows[row],
            FIRST_ROW + slot as f32 * ROW_H,
        );
    }
    // The scroll bar (`Scrollbar Vertical`): the track, the handle in proportion; pressing the track pages.
    frame.rect(
        d.at(SCROLL[0], SCROLL[1]),
        d.at(SCROLL[2], SCROLL[3]),
        [82, 82, 82, 255],
    );
    let share = VISIBLE_ROWS as f32 / N_GENES as f32;
    let handle_h = SCROLL[3] * share;
    let travel = SCROLL[3] - handle_h;
    let along = view.scroll_rows as f32 / max_scroll_rows() as f32;
    let handle_y = SCROLL[1] + travel * along;
    d.slice(
        frame,
        "scroll",
        3.0,
        1.0,
        [SCROLL[0], handle_y, SCROLL[2], handle_h],
        WHITE,
    );
    let above = [SCROLL[0], SCROLL[1], SCROLL[2], handle_y - SCROLL[1]];
    let below = [
        SCROLL[0],
        handle_y + handle_h,
        SCROLL[2],
        SCROLL[1] + SCROLL[3] - handle_y - handle_h,
    ];
    for (offset, rect, up) in [(0, above, true), (1, below, false)] {
        if rect[3] > 0.0 {
            frame.hit(
                HIT_SCROLL + offset,
                d.at(rect[0], rect[1]),
                d.at(rect[2], rect[3]),
                UiAction::Editor(EditorAction::Page(up)),
            );
        }
    }
}

/// One gene's row: the name, the value (or its edit field), the slider with the default pointer, and the reset button.
fn gene_row(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    app: &App,
    session: &EditorSession,
    gene: usize,
    top: f32,
) {
    let model = &session.model;
    let info = gene_info(gene);
    let def = &GENE_DEFS[gene];
    let value = model.genes()[gene];
    let center = top + ROW_H / 2.0;
    let name_px = 28.0 * d.s;
    // The name, with the gene's help as its tooltip.
    frame.bold(
        info.name,
        Vec2::new(NAME_X * d.s, center * d.s - 0.6 * name_px),
        name_px,
        WHITE,
    );
    let name_width = d.measure(info.name, name_px, true) / d.s;
    frame.tips.push(TipRect {
        pos: d.at(NAME_X, top),
        size: d.at(name_width, ROW_H),
        tip: Tip {
            path: "",
            title: info.name,
            content: info.help,
            ui: true,
            instant: false,
        },
    });
    // The value text, a button that opens the edit field (`SettingSlider.OpenEditPanel`).
    let editing = app
        .value_edit
        .as_ref()
        .filter(|edit| edit.field == crate::dyn_panel::FieldRef::Gene(gene as u8));
    let value_px = 28.0 * d.s;
    if let Some(edit) = editing {
        let rect = [VALUE_X - 70.0, top + 8.0, 140.0, ROW_H - 16.0];
        d.slice(frame, "InputFieldBackground", 8.0, 1.0, rect, WHITE);
        let text = format!(
            "{}{}",
            edit.text,
            if edit.caret_visible() { "|" } else { "" }
        );
        frame.text(
            text,
            Vec2::new((rect[0] + 8.0) * d.s, center * d.s - 0.6 * value_px),
            value_px * 0.8,
            INK,
        );
        frame.hit(
            HIT_ROWS + gene as UiId * 4,
            d.at(rect[0], rect[1]),
            d.at(rect[2], rect[3]),
            UiAction::ValueEditInside,
        );
    } else {
        let text = gene_format(info).format(value);
        let width = d.measure(&text, value_px, true);
        frame.bold(
            text,
            Vec2::new(VALUE_X * d.s - width / 2.0, center * d.s - 0.6 * value_px),
            value_px,
            WHITE,
        );
        frame.hit(
            HIT_ROWS + gene as UiId * 4,
            d.at(VALUE_X - 70.0, top + 8.0),
            d.at(140.0, ROW_H - 16.0),
            UiAction::DynEditValue(crate::dyn_panel::FieldRef::Gene(gene as u8)),
        );
    }
    // The slider and the reset button hide while the value is typed.
    if editing.is_some() {
        return;
    }
    let track = [TRACK[0], center - 8.0, TRACK[1] - TRACK[0], 16.0];
    d.slice(frame, "Background", 10.0, 1.0, track, [255, 255, 255, 65]);
    let handle = handle_x(gene, value);
    d.slice(
        frame,
        "UISprite",
        10.0,
        1.0,
        [TRACK[0], track[1], handle - TRACK[0] + 6.0, track[3]],
        WHITE,
    );
    // The default value's pointer: a small mark above and below the track.
    let pointer = handle_x(gene, def.default);
    for y in [track[1] - 11.0, track[1] + track[3] + 1.0] {
        frame.rect(d.at(pointer - 2.0, y), d.at(4.0, 10.0), [56, 56, 56, 255]);
    }
    let knob = [handle - 17.0, center - 17.0, 34.0, 34.0];
    d.icon(frame, "base_circle_icon", knob, WHITE);
    frame.hit(
        HIT_ROWS + gene as UiId * 4 + 1,
        d.at(knob[0], knob[1]),
        d.at(knob[2], knob[3]),
        UiAction::Editor(EditorAction::GeneSlide(
            gene as u8,
            TRACK[0] as u16,
            TRACK[1] as u16,
        )),
    );
    frame.hit(
        HIT_ROWS + gene as UiId * 4 + 2,
        d.at(track[0] - 10.0, center - 20.0),
        d.at(track[2] + 20.0, 40.0),
        UiAction::Editor(EditorAction::GeneSlide(
            gene as u8,
            TRACK[0] as u16,
            TRACK[1] as u16,
        )),
    );
    let reset = [RESET_X, top + 8.0, 44.0, 44.0];
    d.slice(frame, "base_icon", 5.0, 1.0 / 0.5, reset, WHITE);
    d.icon(frame, "reset_icon", reset, WHITE);
    frame.hit(
        HIT_ROWS + gene as UiId * 4 + 3,
        d.at(reset[0], reset[1]),
        d.at(reset[2], reset[3]),
        UiAction::Editor(EditorAction::GeneReset(gene as u8)),
    );
}

/// The right column: the preview's frame, the name with its warnings, the description.
fn right_column(frame: &mut UiFrame, d: &Drawer<'_>, app: &App, session: &EditorSession) {
    let (model, view) = (&session.model, &session.view);
    d.slice(frame, "gray_panel", 10.0, 1.0 / 0.5, PREVIEW_FRAME, WHITE);
    frame.rect(
        d.at(PREVIEW_FRAME[0] + 12.0, PREVIEW_FRAME[1] + 12.0),
        d.at(PREVIEW_FRAME[2] - 24.0, PREVIEW_FRAME[3] - 24.0),
        [0, 0, 27, 255],
    );
    // The template's procedural bibite with the food icons (`ProceduralGenePreviewer`), when the game's sprites are loaded.
    if let Some(texture) = app.editor_picture([PREVIEW_FRAME[2], PREVIEW_FRAME[3]], d.s) {
        frame.draws.push(UiDraw::Pixels {
            texture,
            pos: d.at(PREVIEW_FRAME[0], PREVIEW_FRAME[1]),
            size: d.at(PREVIEW_FRAME[2], PREVIEW_FRAME[3]),
        });
    }
    // While the Brain tab shows, what happens to the nodes' places is said at the top (`positionSavedMessages`).
    if view.tab == Tab::Brain {
        crate::brain_editor_ui::saved_notice(frame, d, session.brain.save_anchors, PREVIEW_FRAME);
    }
    d.slice(frame, "base_icon", 5.0, 1.0 / 0.5, INFO_BUTTON, WHITE);
    d.icon(frame, "information_icon_0", INFO_BUTTON, WHITE);
    frame.hit(
        HIT_INFO,
        d.at(INFO_BUTTON[0], INFO_BUTTON[1]),
        d.at(INFO_BUTTON[2], INFO_BUTTON[3]),
        UiAction::Editor(EditorAction::Info),
    );
    d.slice(frame, "gray_panel", 10.0, 1.0 / 0.5, BODY_FRAME, WHITE);
    if view.info_open {
        stats_panel(frame, d, model);
        return;
    }
    let label_px = 24.0 * d.s;
    frame.bold(
        "Name:",
        Vec2::new(1388.0 * d.s, WARNING_Y * d.s - 0.6 * label_px),
        label_px,
        WHITE,
    );
    text_field(
        frame,
        d,
        NAME_FIELD,
        model.name(),
        view,
        TextField::Name,
        "Enter name...",
    );
    // The warning icons that show (`bug_icon`: red for a bad character and an official name, orange for an existing file).
    let shown = warnings(model);
    let tints: [Rgba; 3] = [[229, 0, 0, 255], [229, 0, 0, 255], [229, 153, 0, 255]];
    for index in 0..3 {
        if !shown[index] {
            continue;
        }
        let rect = [
            WARNING_X[index] - WARNING_SIZE / 2.0,
            WARNING_Y - WARNING_SIZE / 2.0,
            WARNING_SIZE,
            WARNING_SIZE,
        ];
        d.icon(frame, "bug_icon", rect, tints[index]);
        if let Some(tip) = crate::tooltip::find(WARNING_TIPS[index]) {
            frame.tips.push(TipRect {
                pos: d.at(rect[0], rect[1]),
                size: d.at(rect[2], rect[3]),
                tip,
            });
        }
    }
    frame.bold(
        "Description:",
        Vec2::new(1388.0 * d.s, 575.0 * d.s - 0.6 * label_px),
        label_px,
        WHITE,
    );
    text_field(
        frame,
        d,
        DESC_FIELD,
        &model.description,
        view,
        TextField::Description,
        "Enter description...",
    );
    if !view.status.is_empty() {
        frame.text(
            view.status.clone(),
            Vec2::new(1388.0 * d.s, 1044.0 * d.s),
            18.0 * d.s,
            WHITE,
        );
    }
    let _ = app;
}

/// The stats panel (`Bibite Stats Preview`): the growth curve with its axes, then the maturity, size and length and the organs' shares of
/// the area, all for the genes as they are.
fn stats_panel(frame: &mut UiFrame, d: &Drawer<'_>, model: &TemplateEditor) {
    use bibites_core::format::FloatValueFormat;
    let preview = model.preview();
    let px = 22.0 * d.s;
    let small = 12.0 * d.s;
    let ink: Rgba = [20, 20, 20, 255];
    // The title: "Bibite Growth Curve (u²/s)".
    let title = "Bibite Growth Curve (u\u{b2}/s)";
    let width = d.measure(title, px, true);
    frame.bold(
        title,
        Vec2::new(
            (BODY_FRAME[0] + BODY_FRAME[2] / 2.0) * d.s - width / 2.0,
            495.0 * d.s,
        ),
        px,
        WHITE,
    );
    // The graph: 11 x 5 lines on a grey body, the curve in green.
    let body = [1400.0, 530.0, 480.0, 210.0];
    let plot = [1470.0, 540.0, 396.0, 155.0];
    frame.rect(
        d.at(body[0], body[1]),
        d.at(body[2], body[3]),
        [84, 84, 84, 255],
    );
    let (x_min, x_max) = (
        preview.curve.first().map_or(0.0, |point| point.0),
        preview.curve.last().map_or(1.0, |point| point.0),
    );
    let top = preview
        .curve
        .iter()
        .map(|point| point.1)
        .fold(0.0_f32, f32::max);
    let y_max = bibites_core::datalog::graph_axis_max(top);
    let x_format = FloatValueFormat {
        si: false,
        precision: 2,
        ..FloatValueFormat::default()
    };
    let y_format = FloatValueFormat {
        units: "u\u{b2}/s",
        si: false,
        precision: 2,
        precision_is_si: true,
        ..FloatValueFormat::default()
    };
    let grid: Rgba = [8, 8, 8, 255];
    for k in 0..11 {
        let along = k as f32 / 10.0;
        let x = plot[0] + plot[2] * along;
        frame.rect(d.at(x, plot[1]), d.at(1.5, plot[3]), grid);
        let text = x_format.format(x_min + (x_max - x_min) * along);
        let w = d.measure(&text, small, false);
        frame.text(
            text,
            Vec2::new(x * d.s - w / 2.0, (plot[1] + plot[3] + 4.0) * d.s),
            small,
            ink,
        );
    }
    for k in 0..5 {
        let along = k as f32 / 4.0;
        let y = plot[1] + plot[3] * (1.0 - along);
        frame.rect(d.at(plot[0], y), d.at(plot[2], 1.5), grid);
        let text = y_format.format(y_max * along);
        let w = d.measure(&text, small, false);
        frame.text(
            text,
            Vec2::new((plot[0] - 6.0) * d.s - w, (y - 6.0) * d.s),
            small,
            ink,
        );
    }
    let x_label = "Maturity";
    let w = d.measure(x_label, small * 1.2, false);
    frame.text(
        x_label,
        Vec2::new(
            (plot[0] + plot[2] / 2.0) * d.s - w / 2.0,
            (plot[1] + plot[3] + 22.0) * d.s,
        ),
        small * 1.2,
        ink,
    );
    // The curve (`SingleLineGraph.SetCurve`), a 2 px dotted line in green.
    let to_screen = |(x, y): (f32, f32)| {
        Vec2::new(
            (plot[0] + plot[2] * (x - x_min) / (x_max - x_min).max(1e-9)) * d.s,
            (plot[1] + plot[3] * (1.0 - (y / y_max).clamp(0.0, 1.0))) * d.s,
        )
    };
    for pair in preview.curve.windows(2) {
        let (a, b) = (to_screen(pair[0]), to_screen(pair[1]));
        let steps = ((b - a).length() / 2.0).ceil().max(1.0) as usize;
        for step in 0..=steps {
            let p = a + (b - a) * (step as f32 / steps as f32);
            frame.rect(p - Vec2::splat(1.0), Vec2::splat(2.0), [0, 255, 0, 255]);
        }
    }
    // The numbers: label on the left, value on the right.
    let percentage = FloatValueFormat::percentage();
    let rows: [(&str, String); 3] = [
        (
            "Maturity at Birth:",
            FloatValueFormat {
                si: false,
                precision_is_si: true,
                precision: 2,
                ..FloatValueFormat::default()
            }
            .format(preview.maturity_at_birth),
        ),
        (
            "Size at Maturity:",
            FloatValueFormat {
                units: "u\u{b2}",
                si: false,
                precision_is_si: true,
                precision: 2,
                ..FloatValueFormat::default()
            }
            .format(preview.size_at_maturity),
        ),
        (
            "Length at Maturity:",
            FloatValueFormat {
                units: "u",
                si: true,
                precision_is_si: true,
                precision: 2,
                ..FloatValueFormat::default()
            }
            .format(preview.length_at_maturity),
        ),
    ];
    let organs: [(&str, f32); 7] = [
        ("Womb Area Portion:", preview.womb),
        ("Stomach Area Portion:", preview.stomach),
        ("Fat Area Portion:", preview.fat),
        ("Armor Area Portion:", preview.armor),
        ("Throat Area Portion:", preview.throat),
        ("Jaw Area Portion:", preview.jaw),
        ("Move Muscles Area Portion:", preview.move_muscles),
    ];
    let line = 22.0;
    let mut y = 752.0;
    let row_px = 19.0 * d.s;
    let put = |frame: &mut UiFrame, label: &str, value: &str, y: f32| {
        frame.bold(label, Vec2::new(1396.0 * d.s, y * d.s), row_px, WHITE);
        let w = d.measure(value, row_px, true);
        frame.bold(value, Vec2::new(1872.0 * d.s - w, y * d.s), row_px, WHITE);
    };
    for (label, value) in &rows {
        put(frame, label, value, y);
        y += line;
    }
    let heading = "Internal Organs Area Distribution";
    let w = d.measure(heading, row_px, true);
    frame.bold(
        heading,
        Vec2::new(
            (BODY_FRAME[0] + BODY_FRAME[2] / 2.0) * d.s - w / 2.0,
            (y + 8.0) * d.s,
        ),
        row_px,
        WHITE,
    );
    y += line + 10.0;
    for (label, share) in &organs {
        put(frame, label, &percentage.format(*share), y);
        y += line;
    }
}

/// A white text field (`TMP_InputField` on `InputFieldBackground`): the text, the caret while it has the focus, the placeholder while empty.
fn text_field(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    rect: [f32; 4],
    text: &str,
    view: &EditorView,
    field: TextField,
    placeholder: &str,
) {
    d.slice(frame, "InputFieldBackground", 8.0, 1.0, rect, WHITE);
    let px = 22.0 * d.s;
    let focused = view.focus == Some(field);
    let caret = focused && (view.idle / 0.85) as u32 % 2 == 0;
    let room = (rect[2] - 16.0) * d.s;
    // The caret bar goes where the caret is (blinking), inside the text before it is wrapped.
    let with_caret;
    let text = if caret {
        with_caret = crate::text_field::with_caret(text);
        with_caret.as_str()
    } else {
        text
    };
    let mut lines: Vec<String> = if text.is_empty() {
        vec![String::new()]
    } else {
        crate::tooltip::wrap(text, room, &|line: &str| d.measure(line, px, false))
    };
    if text.ends_with('\n') {
        lines.push(String::new());
    }
    if text.is_empty() && !focused {
        frame.text(
            placeholder.to_owned(),
            Vec2::new((rect[0] + 8.0) * d.s, (rect[1] + 8.0) * d.s),
            px,
            [150, 150, 150, 255],
        );
    }
    let line_h = px * 1.2;
    let last = lines.len().saturating_sub(1);
    for (index, line) in lines.iter().enumerate() {
        let y = (rect[1] + 8.0) * d.s + index as f32 * line_h;
        if y + line_h > (rect[1] + rect[3]) * d.s {
            break;
        }
        let shown = line.clone();
        let _ = last;
        if !shown.is_empty() {
            frame.text(shown, Vec2::new((rect[0] + 8.0) * d.s, y), px, INK);
        }
    }
    frame.hit(
        if field == TextField::Name {
            HIT_NAME
        } else {
            HIT_DESC
        },
        d.at(rect[0], rect[1]),
        d.at(rect[2], rect[3]),
        UiAction::Editor(EditorAction::Focus(field)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{FrameInput, LoadedWorld};
    use crate::dyn_panel::FieldRef;
    use crate::ui::UiDraw;
    use bibites_io::storage::StoragePaths;

    /// An app with a selected bibite, its editor open and a throwaway templates folder.
    fn app_with_editor(tag: &str) -> (App, std::path::PathBuf) {
        let loaded = LoadedWorld::load(None, 31, 8, 1).expect("starter world");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        let dir =
            std::env::temp_dir().join(format!("rustibites_editor_ui_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        app.storage = StoragePaths::new(dir.clone());
        app.select_first();
        app.open_template_editor_for_selection();
        (app, dir)
    }

    fn frame_of(app: &App) -> UiFrame {
        let mut frame = UiFrame::new(1.0);
        draw(&mut frame, app, &UiAssets::empty(), None, 1920.0, 1080.0);
        frame
    }

    fn texts(frame: &UiFrame) -> Vec<String> {
        frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.trim().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_editor_opens_on_the_selected_bibite_with_its_genes_and_a_name() {
        let (app, dir) = app_with_editor("open");
        let session = app.bibite_editor.as_ref().expect("the editor is open");
        assert!(app.panels.is_open(crate::ui::PanelKind::BibiteEditor));
        let id = app.selected.expect("selected");
        let record = app.world.bibites.get(id).expect("bibite");
        // Its genes (kept inside their ranges), a name that is not empty, a template that can be saved.
        for (index, (gene, def)) in session
            .model
            .genes()
            .iter()
            .zip(GENE_DEFS.iter())
            .enumerate()
        {
            assert_eq!(
                *gene,
                record.genome.genes[index].clamp(def.min, def.max),
                "{}",
                def.name
            );
        }
        assert!(!session.model.name().is_empty());
        assert!(session.model.can_save());
        assert_eq!(session.view.tab, Tab::Genes);
        assert!(!session.model.any_change);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_genes_tab_lists_sixteen_rows_and_scrolls_a_row_a_notch() {
        let (mut app, dir) = app_with_editor("rows");
        let shown = texts(&frame_of(&app));
        for label in [
            "Bibite Editor",
            "Red Color",
            "Diet",
            "Lay Time",
            "View Angle",
            "Name:",
            "Description:",
        ] {
            assert!(shown.contains(&label.to_owned()), "{label}: {shown:?}");
        }
        assert!(
            !shown.contains(&"Fat Storage Deadband".to_owned()),
            "the last rows are out of the list"
        );
        // The wheel scrolls a row a notch (down is negative), up to the last row.
        app.editor_wheel(Vec2::new(600.0, 500.0), -1.0);
        let shown = texts(&frame_of(&app));
        assert!(
            !shown.contains(&"Red Color".to_owned()) && shown.contains(&"Green Color".to_owned())
        );
        for _ in 0..30 {
            app.editor_wheel(Vec2::new(600.0, 500.0), -1.0);
        }
        assert_eq!(
            app.bibite_editor.as_ref().unwrap().view.scroll_rows,
            max_scroll_rows()
        );
        let shown = texts(&frame_of(&app));
        assert!(
            shown.contains(&"Fat Storage Deadband".to_owned())
                && !shown.contains(&"Diet".to_owned())
        );
        // The scroll bar's track pages the list.
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::Page(true))]);
        assert_eq!(
            app.bibite_editor.as_ref().unwrap().view.scroll_rows,
            max_scroll_rows() - VISIBLE_ROWS
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_slider_follows_the_cursor_and_the_reset_button_goes_back_to_the_default() {
        let (mut app, dir) = app_with_editor("slider");
        let diet = GENE_DEFS.iter().position(|def| def.name == "Diet").unwrap();
        // A press on the track sets the value there; the drag goes on; past the ends it stops at them.
        app.editor_gene_press(
            diet as u8,
            (TRACK[0], TRACK[1]),
            (TRACK[0] + TRACK[1]) / 2.0,
        );
        let model = &app.bibite_editor.as_ref().unwrap().model;
        assert!((model.genes()[diet] - 0.5).abs() < 1e-3);
        assert!(model.any_change);
        assert!(app.editor_gene_dragging());
        app.editor_gene_slide(TRACK[0] + (TRACK[1] - TRACK[0]) * 0.25);
        assert!((app.bibite_editor.as_ref().unwrap().model.genes()[diet] - 0.25).abs() < 1e-3);
        app.editor_gene_slide(-100.0);
        assert_eq!(app.bibite_editor.as_ref().unwrap().model.genes()[diet], 0.0);
        app.editor_gene_slide(5000.0);
        assert_eq!(app.bibite_editor.as_ref().unwrap().model.genes()[diet], 1.0);
        app.editor_gene_release();
        assert!(!app.editor_gene_dragging());
        // The reset button of the row.
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::GeneReset(diet as u8))]);
        assert_eq!(
            app.bibite_editor.as_ref().unwrap().model.genes()[diet],
            GENE_DEFS[diet].default
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_typed_value_goes_into_the_gene_inside_its_range_and_shows_in_its_format() {
        let (mut app, dir) = app_with_editor("typed");
        let lay = GENE_DEFS
            .iter()
            .position(|def| def.name == "LayTime")
            .unwrap();
        let field = FieldRef::Gene(lay as u8);
        // The row's value text opens the field with the value times the factor, selected.
        app.apply_ui_actions(&[UiAction::DynEditValue(field)]);
        assert!(
            frame_of(&app)
                .actions
                .iter()
                .any(|(_, a)| *a == UiAction::ValueEditInside)
        );
        app.typed = "33.5".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            enter: true,
            ..FrameInput::default()
        });
        let value = app.bibite_editor.as_ref().unwrap().model.genes()[lay];
        assert!((value - 33.5).abs() < 1e-3, "{value}");
        assert!(texts(&frame_of(&app)).contains(&"33.5 s".to_owned()));
        // A number past the end stops at the end (the gene never leaves its range).
        app.apply_ui_actions(&[UiAction::DynEditValue(field)]);
        app.typed = "9999".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            enter: true,
            ..FrameInput::default()
        });
        assert_eq!(
            app.bibite_editor.as_ref().unwrap().model.genes()[lay],
            GENE_DEFS[lay].max
        );
        // Escape closes the field without applying it.
        app.apply_ui_actions(&[UiAction::DynEditValue(field)]);
        app.typed = "1".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            quit: true,
            ..FrameInput::default()
        });
        assert!(app.value_edit.is_none());
        assert_eq!(
            app.bibite_editor.as_ref().unwrap().model.genes()[lay],
            GENE_DEFS[lay].max
        );
        assert!(
            app.bibite_editor.is_some(),
            "the first Escape only closed the field"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_name_and_the_description_are_typed_in_with_their_warnings() {
        let (mut app, dir) = app_with_editor("text");
        // Nothing focused: typed characters are dropped.
        app.typed = "zzz".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.typed.is_empty());
        let original = app.bibite_editor.as_ref().unwrap().model.name().to_owned();
        // A press on the name field focuses it; typing adds to the name, Backspace takes away.
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::Focus(TextField::Name))]);
        app.typed = "A/b".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
        let model = &app.bibite_editor.as_ref().unwrap().model;
        assert_eq!(model.name(), format!("{original}A/b"));
        // A character a file name cannot have: the red warning shows, Save is greyed.
        assert!(model.check.bad_character && !model.can_save());
        assert!(warnings(model)[0]);
        app.handle_input(&FrameInput {
            dt: 0.016,
            backspace: true,
            ..FrameInput::default()
        });
        app.handle_input(&FrameInput {
            dt: 0.016,
            backspace: true,
            ..FrameInput::default()
        });
        assert!(app.bibite_editor.as_ref().unwrap().model.can_save());
        // Enter leaves the name field; the description takes new lines.
        app.handle_input(&FrameInput {
            dt: 0.016,
            enter: true,
            ..FrameInput::default()
        });
        assert!(app.bibite_editor.as_ref().unwrap().view.focus.is_none());
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::Focus(
            TextField::Description,
        ))]);
        app.typed = "one".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            enter: true,
            ..FrameInput::default()
        });
        app.typed = "two".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
        assert_eq!(
            app.bibite_editor.as_ref().unwrap().model.description,
            "one\ntwo"
        );
        // Escape leaves the field first, then closes the editor.
        app.handle_input(&FrameInput {
            dt: 0.016,
            quit: true,
            ..FrameInput::default()
        });
        assert!(
            app.bibite_editor.is_some() && app.bibite_editor.as_ref().unwrap().view.focus.is_none()
        );
        app.handle_input(&FrameInput {
            dt: 0.016,
            quit: true,
            ..FrameInput::default()
        });
        assert!(app.bibite_editor.is_none());
        assert!(!app.panels.is_open(crate::ui::PanelKind::BibiteEditor));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn save_writes_the_template_and_stays_and_save_and_submit_closes() {
        let (mut app, dir) = app_with_editor("save");
        let diet = GENE_DEFS.iter().position(|def| def.name == "Diet").unwrap();
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::Focus(TextField::Name))]);
        // A fresh name: replace the one it has.
        let old = app.bibite_editor.as_ref().unwrap().model.name().len();
        for _ in 0..old {
            app.handle_input(&FrameInput {
                dt: 0.016,
                backspace: true,
                ..FrameInput::default()
            });
        }
        app.typed = "Edited one".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::GeneReset(diet as u8))]);
        app.editor_gene_press(diet as u8, (TRACK[0], TRACK[1]), TRACK[1]);
        app.editor_gene_release();
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::Save)]);
        let path = dir
            .join("Bibites")
            .join("Templates")
            .join("Edited one.bb8template");
        assert!(path.is_file(), "{}", path.display());
        assert!(app.bibite_editor.is_some(), "Save and continue stays");
        assert!(
            app.bibite_editor
                .as_ref()
                .unwrap()
                .view
                .status
                .starts_with("Saved")
        );
        let loaded = bibites_io::load_template(&path).expect("reads back");
        assert_eq!(loaded.genome.genes[diet], 1.0);
        // The name is taken now: the orange warning shows. Save and submit writes again and closes the editor.
        assert!(warnings(&app.bibite_editor.as_ref().unwrap().model)[2]);
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::Submit)]);
        assert!(
            app.bibite_editor.is_none() && !app.panels.is_open(crate::ui::PanelKind::BibiteEditor)
        );
        // `AfterEditorSubmit`: the Library under it fills its lists again and selects the template that was saved.
        assert_eq!(app.library_refill, Some(Some("Edited one".to_owned())));
        // A name that cannot be saved under is not written and the editor stays.
        app.open_template_editor_for_selection();
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::Focus(TextField::Name))]);
        app.typed = "/".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
        let before = std::fs::read_dir(dir.join("Bibites").join("Templates"))
            .unwrap()
            .count();
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::Save)]);
        assert_eq!(
            std::fs::read_dir(dir.join("Bibites").join("Templates"))
                .unwrap()
                .count(),
            before
        );
        assert!(app.bibite_editor.is_some());
        // The Back button closes it without saving; the Library fills its lists again and keeps what it had selected
        // (`AfterEditorClose`).
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::Back)]);
        assert!(app.bibite_editor.is_none());
        assert_eq!(app.library_refill, Some(None));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The rectangle of the control of `action` in a frame (reference px at scale 1).
    fn rect_of(frame: &UiFrame, action: UiAction) -> Option<[f32; 4]> {
        let id = frame
            .actions
            .iter()
            .find(|(_, candidate)| *candidate == action)
            .map(|(id, _)| *id)?;
        let (_, pos, size) = frame.hits.iter().find(|hit| hit.0 == id)?;
        Some([pos.x, pos.y, size.x, size.y])
    }

    #[test]
    fn the_inspectors_edit_opens_the_library_with_the_editor_over_it_and_pauses_the_game() {
        use crate::ui::PanelKind;
        let (mut app, dir) = app_with_editor("library");
        // `OpenForBibiteEditor`: the Library opens, then the editor over it.
        assert!(
            app.panels.is_open(PanelKind::Library) && app.panels.is_open(PanelKind::BibiteEditor)
        );
        assert_eq!(app.panels.top(), Some(PanelKind::BibiteEditor));
        assert!(app.bibite_editor.as_ref().unwrap().submit);
        // The Library pauses the game (`TogglePauseGame("BibiteLibrary")`) and gives it back as it was when it closes.
        app.paused = false;
        app.advance(0.05);
        assert!(app.paused, "paused by the Library and the editor");
        // Escape closes the editor and lands in the Library, which fills its lists again; the game stays paused.
        app.handle_input(&FrameInput {
            quit: true,
            ..FrameInput::default()
        });
        assert!(app.bibite_editor.is_none());
        assert!(app.panels.is_open(PanelKind::Library));
        assert_eq!(app.panels.top(), Some(PanelKind::Library));
        assert_eq!(app.library_refill, Some(None));
        app.advance(0.05);
        assert!(app.paused);
        // The next Escape closes the Library and the game runs again.
        app.handle_input(&FrameInput {
            quit: true,
            ..FrameInput::default()
        });
        assert!(!app.panels.is_open(PanelKind::Library));
        app.advance(0.05);
        assert!(!app.paused);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_pause_menu_that_opened_the_library_does_not_cover_the_editor_over_it() {
        use crate::ui::{ControlCommand, PanelKind};
        let (mut app, dir) = app_with_editor("pause");
        app.close_template_editor();
        app.panels.close(PanelKind::Library);
        // Pause menu -> Bibite Library -> Edit Bibite: `EscapePanel` stays active under both (nothing closes it).
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenEscapeMenu)]);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenLibrary)]);
        app.open_editor_from_library(None, false);
        assert!(app.panels.is_open(PanelKind::Escape));
        assert_eq!(app.panels.top(), Some(PanelKind::BibiteEditor));
        let extra_panels = |app: &App| {
            let mut frame = UiFrame::new(1.0);
            crate::hud::draw_extra_panels(
                &mut frame,
                app,
                &UiAssets::empty(),
                None,
                1920.0,
                1080.0,
                false,
            );
            frame
        };
        let pause_menu_button = UiAction::Invoke(ControlCommand::OpenSavePanel);
        let covered = extra_panels(&app);
        // The menu draws nothing and takes no click: the editor's own controls answer (it used to sit on top of the editor, and its
        // full-screen click catcher swallowed every press meant for it).
        assert!(rect_of(&covered, pause_menu_button).is_none());
        let back = rect_of(&covered, UiAction::Editor(EditorAction::Back)).expect("Back");
        let middle = Vec2::new(back[0] + back[2] * 0.5, back[1] + back[3] * 0.5);
        assert_eq!(
            covered.action_at(middle),
            Some(UiAction::Editor(EditorAction::Back))
        );
        // Escape closes the editor: the Library is the newest panel, the menu still hidden.
        app.handle_input(&FrameInput {
            quit: true,
            ..FrameInput::default()
        });
        assert_eq!(app.panels.top(), Some(PanelKind::Library));
        assert!(rect_of(&extra_panels(&app), pause_menu_button).is_none());
        // Escape closes the Library: the pause menu is back, with its buttons.
        app.handle_input(&FrameInput {
            quit: true,
            ..FrameInput::default()
        });
        assert_eq!(app.panels.top(), Some(PanelKind::Escape));
        assert!(rect_of(&extra_panels(&app), pause_menu_button).is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pick_in_the_library_goes_on_after_the_editor() {
        use crate::ui::PanelKind;
        // The Library opened to pick a template for the placer keeps its own pause; the editor over it does not end the pick.
        let (mut app, dir) = app_with_editor("pick");
        app.close_template_editor();
        app.panels.close(PanelKind::Library);
        app.paused = false;
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::LoadBibite)]);
        assert!(app.paused && app.template_pick.is_some());
        app.open_editor_from_library(None, false);
        assert!(app.bibite_editor.is_some() && app.template_pick.is_some());
        app.advance(0.05);
        app.close_template_editor();
        app.advance(0.05);
        assert!(app.paused, "still picking");
        assert!(app.panels.is_open(PanelKind::Library));
        app.cancel_template_pick();
        assert!(!app.paused, "the placer's pause ends with the pick");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn create_new_opens_a_blank_template_without_save_and_submit() {
        let (mut app, dir) = app_with_editor("create");
        let with_submit = frame_of(&app);
        let submit = rect_of(&with_submit, UiAction::Editor(EditorAction::Submit)).expect("Submit");
        let save = rect_of(&with_submit, UiAction::Editor(EditorAction::Save)).expect("Save");
        let sprite =
            rect_of(&with_submit, UiAction::Editor(EditorAction::SaveSprite)).expect("Save sprite");
        let back = rect_of(&with_submit, UiAction::Editor(EditorAction::Back)).expect("Back");
        // `OpenEditorForNewBibite`: `EditTemplate(null)`: the genes' defaults, the editor's name, the bare base brain.
        app.close_template_editor();
        app.open_editor_from_library(None, false);
        let session = app.bibite_editor.as_ref().expect("open");
        assert!(!session.submit);
        assert_eq!(session.model.name(), "Bibite template");
        assert!(!session.model.any_change);
        for (gene, def) in session.model.genes().iter().zip(GENE_DEFS.iter()) {
            assert_eq!(*gene, def.default, "{}", def.name);
        }
        assert_eq!(session.model.brain.node_count(), 33 + 15);
        assert_eq!(session.model.brain.synapse_count(), 0);
        // `saveAndSubmitButton.SetActive(false)`: no Submit, and the layout group packs the buttons that were left of it 80 over.
        let frame = frame_of(&app);
        assert!(rect_of(&frame, UiAction::Editor(EditorAction::Submit)).is_none());
        assert_eq!(
            rect_of(&frame, UiAction::Editor(EditorAction::Save)).map(|r| r[0]),
            Some(save[0] + 80.0)
        );
        assert_eq!(
            rect_of(&frame, UiAction::Editor(EditorAction::SaveSprite)).map(|r| r[0]),
            Some(sprite[0] + 80.0)
        );
        assert_eq!(
            rect_of(&frame, UiAction::Editor(EditorAction::Back)),
            Some(back)
        );
        // The Submit button sat where Save is now (to within a pixel or two of the measured offsets).
        assert!((submit[0] - (save[0] + 80.0)).abs() < 3.0);
        // Save still works and stays; "Save sprite" says it is not available.
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::SaveSprite)]);
        assert!(
            app.bibite_editor
                .as_ref()
                .unwrap()
                .view
                .status
                .contains("not available")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_library_opens_the_editor_on_a_template_file_and_a_file_that_does_not_read_opens_nothing()
    {
        let (mut app, dir) = app_with_editor("file");
        app.close_template_editor();
        let folder = dir.join("Bibites").join("Templates");
        std::fs::create_dir_all(&folder).expect("folder");
        let mut loaded = bibites_io::LoadedGenome::starter_template("From the library");
        loaded.genome.genes[GENE_DEFS.iter().position(|def| def.name == "Diet").unwrap()] = 0.7;
        let path = folder.join("a file.bb8template");
        bibites_io::save_template(
            &path,
            &loaded.genome,
            &loaded.meta,
            &loaded.brain,
            &loaded.template,
        )
        .expect("writes");
        // `EditSelectedBibite`: the template's genes and name, with "Save and submit".
        app.open_editor_from_library(Some(&path), true);
        let session = app.bibite_editor.as_ref().expect("open");
        assert_eq!(session.model.name(), "From the library");
        assert!(session.submit);
        let diet = GENE_DEFS.iter().position(|def| def.name == "Diet").unwrap();
        assert_eq!(session.model.genes()[diet], 0.7);
        // A file that is gone opens nothing (the Library would have shown its parser error and dropped the item).
        app.close_template_editor();
        app.open_editor_from_library(Some(&folder.join("gone.bb8template")), true);
        assert!(app.bibite_editor.is_none());
        assert!(
            app.last_action.starts_with("could not read"),
            "{}",
            app.last_action
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_species_panels_edit_opens_the_editor_on_the_species_template() {
        use crate::ui::PanelKind;
        let (mut app, dir) = app_with_editor("species");
        app.close_template_editor();
        app.panels.close(PanelKind::Library);
        app.world.run(50);
        let (genome, brain) = {
            let (_, record) = app.world.bibites.iter().next().expect("a bibite");
            (record.genome.clone(), record.brain.clone())
        };
        let id = app
            .world
            .species
            .add_template_species("Zzedit zzspecies", "", &genome, &brain);
        app.refresh_lineage();
        app.panels.open(PanelKind::Lineage);
        app.select_species_info(Some(id));
        app.apply_ui_actions(&[UiAction::SpeciesEdit]);
        // `new BibiteTemplate(selectedSpecies.template)`: named after the species, its genes and its brain, in the editor over the
        // Library (which is over the species panel).
        let session = app.bibite_editor.as_ref().expect("the editor opened");
        assert_eq!(
            session.model.name(),
            app.world.species.get(id).unwrap().name()
        );
        assert_eq!(session.model.brain.node_count(), brain.node_count());
        assert!(session.submit);
        assert_eq!(app.panels.top(), Some(PanelKind::BibiteEditor));
        assert!(app.panels.is_open(PanelKind::Library) && app.panels.is_open(PanelKind::Lineage));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_info_button_opens_the_stats_panel_over_the_name_and_the_description() {
        let (mut app, dir) = app_with_editor("info");
        let before = texts(&frame_of(&app));
        assert!(before.contains(&"Description:".to_owned()));
        assert!(!before.iter().any(|text| text.contains("Growth Curve")));
        let info = frame_of(&app)
            .actions
            .iter()
            .any(|(_, action)| *action == UiAction::Editor(EditorAction::Info));
        assert!(info, "the info button is on the preview");
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::Info)]);
        let frame = frame_of(&app);
        let shown = texts(&frame);
        for label in [
            "Bibite Growth Curve (u\u{b2}/s)",
            "Maturity",
            "Maturity at Birth:",
            "Size at Maturity:",
            "Length at Maturity:",
            "Internal Organs Area Distribution",
            "Womb Area Portion:",
            "Stomach Area Portion:",
            "Fat Area Portion:",
            "Armor Area Portion:",
            "Throat Area Portion:",
            "Jaw Area Portion:",
            "Move Muscles Area Portion:",
        ] {
            assert!(shown.contains(&label.to_owned()), "{label}: {shown:?}");
        }
        // It covers the fields: no name or description to press, no warning icons.
        assert!(!shown.contains(&"Description:".to_owned()));
        assert!(
            frame
                .actions
                .iter()
                .all(|(_, action)| !matches!(action, UiAction::Editor(EditorAction::Focus(_))))
        );
        // The same button closes it.
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::Info)]);
        assert!(texts(&frame_of(&app)).contains(&"Description:".to_owned()));
        // The picture is the game's sprites'; the style is the editor's previewer (a 548 x 392 frame, the food icons 214 right).
        assert!(
            app.editor_picture([548.0, 392.0], 1.0).is_none(),
            "no game sprites in a test"
        );
        assert_eq!(crate::preview::EDITOR.food, Some([214.0, 40.0, 40.0]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_hotkeys_wait_while_the_editor_is_open_and_the_save_buttons_grey_with_a_bad_name() {
        let (mut app, dir) = app_with_editor("keys");
        let paused = app.paused;
        app.handle_input(&FrameInput {
            dt: 0.016,
            toggle_pause: true,
            ..FrameInput::default()
        });
        assert_eq!(
            app.paused, paused,
            "Space is not the game's while the editor is open"
        );
        // The buttons: the two tabs, Save and continue, Save and submit, Back.
        let frame = frame_of(&app);
        let editor_actions: Vec<EditorAction> = frame
            .actions
            .iter()
            .filter_map(|(_, action)| match action {
                UiAction::Editor(action) => Some(*action),
                _ => None,
            })
            .collect();
        for expected in [
            EditorAction::Tab(Tab::Genes),
            EditorAction::Tab(Tab::Brain),
            EditorAction::Save,
            EditorAction::Submit,
            EditorAction::Back,
        ] {
            assert!(editor_actions.contains(&expected), "{expected:?}");
        }
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::Focus(TextField::Name))]);
        app.typed = "?".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
        let frame = frame_of(&app);
        assert!(
            frame.actions.iter().all(|(_, action)| !matches!(
                action,
                UiAction::Editor(EditorAction::Save | EditorAction::Submit)
            )),
            "Save and Save and submit work only with a valid name"
        );
        // The Brain tab shows what happens to the nodes' places over the preview.
        app.apply_ui_actions(&[UiAction::Editor(EditorAction::Tab(Tab::Brain))]);
        let shown = texts(&frame_of(&app));
        assert!(
            shown.iter().any(|text| text.contains("be saved with template")),
            "{shown:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_rows_are_the_genes_in_the_settings_order() {
        let rows = row_genes();
        let mut seen = [false; N_GENES];
        for row in rows {
            assert!(!seen[*row], "a gene twice");
            seen[*row] = true;
        }
        assert!(seen.iter().all(|seen| *seen));
        // The list starts with the colours and Diet, as the reference does.
        assert_eq!(gene_info(rows[0]).name, "Red Color");
        assert_eq!(gene_info(rows[4]).name, "Diet");
        assert_eq!(gene_info(rows[11]).name, "Lay Time");
    }

    #[test]
    fn a_slider_maps_the_track_to_the_genes_whole_range() {
        let lay = GENE_DEFS
            .iter()
            .position(|d| d.name == "LayTime")
            .expect("gene");
        let def = &GENE_DEFS[lay];
        assert_eq!(value_at(lay, TRACK[0], TRACK[1], TRACK[0]), def.min);
        assert_eq!(value_at(lay, TRACK[0], TRACK[1], TRACK[1]), def.max);
        let middle = value_at(lay, TRACK[0], TRACK[1], (TRACK[0] + TRACK[1]) / 2.0);
        assert!((middle - (def.min + def.max) / 2.0).abs() < 1e-3);
        // Past the ends it stops at them; the handle goes where the value is and back.
        assert_eq!(value_at(lay, TRACK[0], TRACK[1], -50.0), def.min);
        assert_eq!(value_at(lay, TRACK[0], TRACK[1], 5000.0), def.max);
        let x = handle_x(lay, def.default);
        assert!((value_at(lay, TRACK[0], TRACK[1], x) - def.default).abs() < 1e-3);
    }

    #[test]
    fn typing_follows_the_field_kind() {
        let mut name = String::new();
        for c in "ab\ncd\t".chars() {
            type_into(&mut name, TextField::Name, c);
        }
        assert_eq!(
            name, "abcd",
            "a name takes no new line and no control characters"
        );
        let mut desc = String::new();
        for c in "ab\ncd\t".chars() {
            type_into(&mut desc, TextField::Description, c);
        }
        assert_eq!(desc, "ab\ncd");
    }

    #[test]
    fn the_list_scrolls_by_the_rows_that_do_not_show() {
        assert_eq!(max_scroll_rows(), 19);
    }
}
