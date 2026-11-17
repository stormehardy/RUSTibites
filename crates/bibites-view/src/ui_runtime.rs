//! Runtime-UI loader: the game's exact UI hierarchy from the committed runtime
//! dump (`tools/ui/ui_runtime_atlas.json.gz`, schema
//! `rustibites.assets.ui_runtime_atlas.v1`).
//!
//! The game ships no script typetrees, so the asset-file UI dump alone cannot
//! place images/texts. `tools/ui/ui_runtime_atlas.json.gz` is a joined dump
//! captured from a hidden-window Default world: geometry + resolved
//! Image/TMP/CanvasScaler/layout fields. This module composes those
//! RectTransforms into a screen-space [`UiFrame`] draw list using the same
//! `UiDraw`/`UiAssets` chrome as the curated HUD, so both the headless
//! screenshots and the macroquad window render identical pixels.
//!
//! Design notes:
//! * the raw gzip is decompressed once into a plain-JSON cache under
//!   `target/ui_runtime_cache/`, keyed by the source file's (size, mtime);
//! * only `sessions.default_world` is parsed (the main-menu session is ignored);
//! * the rect composer body is swappable via [`RectSemantics`] because
//!   parity-dev is confirming local-vs-canvas semantics; the default `Local`
//!   path is the validated RectTransform math;
//! * no game assets are required to load/parse the data; unresolvable sprites
//!   are skipped, and the curated [`crate::hud`] remains the fallback when the
//!   data is absent or `--simple-hud` is used.

use std::collections::{BTreeMap, HashSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use bibites_core::math::Vec2;
use serde::Deserialize;

use crate::app::App;
use crate::ui::{
    ControlCommand, ControlValue, FontRole, InspectorTab, OverlayKind, PanelKind, Rgba, ToolKind,
    UiAction, UiAssets, UiDraw, UiFrame, UiId,
};

/// Deserializes a boolean that may be `null` in the capture (treated as false).
fn null_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<bool>::deserialize(deserializer)?.unwrap_or(false))
}

/// Deserializes a list that may be `null` in the capture (treated as empty).
fn null_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

/// Deserializes a number that may be `null` in the capture (treated as 0.0).
fn null_f32<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<f32>::deserialize(deserializer)?.unwrap_or(0.0))
}

/// Deserializes a RectTransform that may be `null` in the capture.
fn null_rect<'de, D>(deserializer: D) -> Result<RectTransform, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<RectTransform>::deserialize(deserializer)?.unwrap_or_default())
}

/// Deserializes a string that may be `null` in the capture (treated as empty).
fn null_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

/// Candidate locations of the joined runtime-UI dump (cwd first, then crate).
const DATA_FILE: &str = "ui_runtime_atlas.json.gz";
/// Reference resolution used by the capture (1920x1080, UIScaler scale 1.0).
const REFERENCE_WIDTH: f32 = 1920.0;
/// Reference height used by the capture.
const REFERENCE_HEIGHT: f32 = 1080.0;

/// How RectTransform anchors are composed into screen space.
///
/// `Local` is the validated math (anchored positions relative to the parent
/// rect, composed down the hierarchy). `Canvas` is the alternate candidate that
/// parity-dev was verifying (canvas-space offsets via `local_position`); it is
/// kept so the composer body can be swapped without touching the draw code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)] // Canvas semantics kept as a parity experiment body
pub enum RectSemantics {
    /// Anchored RectTransform math (default, validated against the runtime dump).
    #[default]
    Local,
    /// Canvas-local offsets (`local_position`), kept as a parity experiment.
    Canvas,
}

/// A composed screen-space rectangle in the 1920x1080 UI reference space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ComposedRect {
    /// Top-left position.
    pub pos: Vec2,
    /// Size.
    pub size: Vec2,
    /// Accumulated lossy scale (for nested scaling).
    pub scale: Vec2,
}

/// Stable id for one captured runtime control (`hash(session, path)`).
pub type ControlId = u64;

/// Control kinds we consider interactable in the runtime UI dump.
#[allow(dead_code)] // P0 registry consumed by the framework follow-up
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    /// Button or generic Selectable.
    Button,
    /// Toggle / checkbox.
    Toggle,
    /// Slider.
    Slider,
    /// Scrollbar.
    Scrollbar,
    /// Scroll view / scroll rect.
    ScrollRect,
    /// Dropdown selector (captured container; the atlas omits the Dropdown component).
    Dropdown,
}

/// Captured value metadata for a toggle/slider/dropdown control.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControlValueMeta {
    /// Toggle state.
    pub is_on: Option<bool>,
    /// Current slider value.
    pub value: Option<f32>,
    /// Minimum slider value.
    pub min_value: Option<f32>,
    /// Maximum slider value.
    pub max_value: Option<f32>,
    /// Slider whole-number mode (implies step 1).
    pub whole_numbers: Option<bool>,
    /// Dropdown option labels.
    pub options: Vec<String>,
    /// Current dropdown selected index.
    pub selected_index: Option<u32>,
    /// Synthesized option row rects `[x,y,w,h]` below the dropdown control.
    /// The atlas does not expose the live popup rects, so these are derived from
    /// the captured dropdown rect at one row-height per option.
    pub option_rects: Vec<[f32; 4]>,
}

/// Captured hotkey text block (the Hotkeys page currently exposes one scroll
/// view containing all key lines, not per-row controls).
#[allow(dead_code)] // captured hotkey text is rendered by the settings panel follow-up
#[derive(Clone, Debug, PartialEq)]
pub struct HotkeyBlock {
    /// Captured node path (normally `UI/SettingsPanel/PagesPanels/Hotkeys/Scroll View`).
    pub path: String,
    /// Text lines from the block, in capture order.
    pub lines: Vec<String>,
}

/// One captured runtime control with its composed screen rect and current
/// mapping. Unmapped controls are exposed here so a future generic
/// `UiAction::ActivateControl(ControlId)` / panel state can bind them without
/// changing the dump.
#[allow(dead_code)] // P0 registry consumed by the framework follow-up
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeControl {
    /// Stable hash of session + path.
    pub id: ControlId,
    /// Runtime session name (e.g. `default_world`).
    pub session: String,
    /// Full captured node path.
    pub path: String,
    /// Node/GameObject name.
    pub name: String,
    /// Visible label (active descendant TMP text), or the node name.
    pub label: String,
    /// Control kind.
    pub kind: ControlKind,
    /// Composed screen rect at the requested size: `[x, y, w, h]` logical px.
    pub rect: [f32; 4],
    /// Value metadata for toggles/sliders/dropdowns.
    pub value: Option<ControlValueMeta>,
    /// Existing `UiAction` mapping, when the behaviour is already clear.
    pub action: Option<UiAction>,
    /// Known menu binding (title menu rows are dispatched by `menu.rs`, not
    /// `UiAction`); exposed for view-dev's panel/action registry migration.
    pub binding: Option<&'static str>,
}

impl RuntimeControl {
    /// Dropdown option labels captured for this control (empty otherwise).
    #[must_use]
    pub fn option_labels(&self) -> &[String] {
        self.value
            .as_ref()
            .map_or(&[], |meta| meta.options.as_slice())
    }

    /// Propose selecting a dropdown option by index.
    #[allow(dead_code)] // TODO(bind): viewer chooses via App::pick_dropdown_choice
    #[allow(dead_code)] // view-dev dropdown panel binding
    #[must_use]
    pub fn proposed_choice(&self, index: usize) -> Option<UiAction> {
        let options = &self.value.as_ref()?.options;
        if index >= options.len() {
            return None;
        }
        Some(UiAction::SetValue(ControlValue::Choice(index as u32)))
    }

    /// Propose the typed value change for a pointer press at `pointer`
    /// (logical pixels): toggles flip, sliders interpolate/clamp to their
    /// rect, dropdowns choose by vertical position. Returns `None` for
    /// non-value controls (buttons) or when captured metadata is missing.
    #[must_use]
    pub fn proposed_value(&self, pointer: Vec2) -> Option<UiAction> {
        let meta = self.value.as_ref()?;
        match self.kind {
            ControlKind::Toggle => meta
                .is_on
                .map(|on| UiAction::SetValue(ControlValue::Flag(!on))),
            ControlKind::Slider => {
                let (min, max) = (meta.min_value?, meta.max_value?);
                if max.partial_cmp(&min) != Some(std::cmp::Ordering::Greater) || self.rect[2] <= 0.0
                {
                    return None;
                }
                let ratio = ((pointer.x - self.rect[0]) / self.rect[2]).clamp(0.0, 1.0);
                let mut value = min + ratio * (max - min);
                if meta.whole_numbers == Some(true) {
                    value = value.round();
                }
                Some(UiAction::SetValue(ControlValue::Float(value)))
            }
            ControlKind::Dropdown => None,
            ControlKind::Scrollbar | ControlKind::ScrollRect | ControlKind::Button => None,
        }
    }
}

/// Parsed runtime UI data with the session and layout helpers needed by frames.
pub struct RuntimeUi {
    /// Composition semantics used when building frames.
    pub semantics: RectSemantics,
    /// Source path of the joined dump.
    pub source: PathBuf,
    /// Parsed `default_world` session (save-state HUD for the current dump).
    pub session: RuntimeSession,
    /// Optional selected-bibite session with the expanded Stats panel.
    pub inspector_session: Option<RuntimeSession>,
    /// Optional selected-bibite Genes tab session.
    pub genes_session: Option<RuntimeSession>,
    /// Optional selected-bibite Biology tab session.
    pub biology_session: Option<RuntimeSession>,
    /// Optional selected-bibite Brain tab session.
    pub brain_session: Option<RuntimeSession>,
    /// Optional title/main-menu session (exact game menu layout).
    pub menu_title: Option<RuntimeSession>,
    /// Optional settings-menu session.
    pub menu_settings: Option<RuntimeSession>,
    /// Functional-UI sessions not covered by the fixed fields above.
    pub extra_sessions: BTreeMap<String, RuntimeSession>,
    /// Sessions parsed on first use from `assets/game/ui_runtime_native/<name>.json.gz`.
    pub lazy_sessions: BTreeMap<String, (PathBuf, std::sync::OnceLock<Option<RuntimeSession>>)>,
    /// Title-menu sessions with the menu-only pruning applied (built once per session).
    filtered_menu: std::sync::Mutex<BTreeMap<String, std::sync::Arc<RuntimeSession>>>,
    /// Last dynamic-panel frame, reused while nothing it shows has changed.
    dyn_cache: std::sync::Mutex<Option<(u64, UiFrame)>>,
    /// Measured HUD ruler value (`ui_save_capture.json` -> `ruler.value`), if any.
    pub ruler_value: Option<f32>,
    /// True when loaded from a native 1920 recapture (all sessions are named).
    pub native: bool,
}

/// One runtime session (`default_world`).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RuntimeSession {
    #[serde(default)]
    /// Captured screen size (reference resolution).
    pub screen: ScreenSize,
    /// Canvas hierarchies (one per captured UI canvas).
    #[serde(default, deserialize_with = "null_vec")]
    pub canvases: Vec<Node>,
    /// Paths of the core HUD/panel nodes the game considers always relevant.
    #[serde(default, deserialize_with = "null_vec")]
    pub core_panel_paths: Vec<String>,
    #[serde(default)]
    /// Number of nodes covered by `core_panel_paths` (from the capture).
    pub core_panel_count: usize,
    #[serde(default)]
    /// Total node count in the session (from the capture).
    pub node_count: usize,
}

/// Captured screen dimensions for a session.
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct ScreenSize {
    #[serde(default = "default_reference_width", deserialize_with = "null_f32")]
    /// Width in pixels.
    pub width: f32,
    #[serde(default = "default_reference_height", deserialize_with = "null_f32")]
    /// Height in pixels.
    pub height: f32,
}

impl Default for ScreenSize {
    fn default() -> Self {
        Self {
            width: default_reference_width(),
            height: default_reference_height(),
        }
    }
}

fn default_reference_width() -> f32 {
    REFERENCE_WIDTH
}

fn default_reference_height() -> f32 {
    REFERENCE_HEIGHT
}

/// One UI node: geometry + resolved components + children.
#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)] // full captured schema; not every field is consumed by the frame builder
pub struct Node {
    #[serde(default)]
    /// GameObject name.
    pub name: String,
    #[serde(default)]
    /// Full hierarchy path.
    pub path: String,
    #[serde(default, deserialize_with = "null_bool")]
    /// Self active flag.
    pub active: bool,
    #[serde(default, deserialize_with = "null_bool")]
    /// Active in the hierarchy (self + all parents).
    pub active_in_hierarchy: bool,
    #[serde(default)]
    /// Sibling order within the parent.
    pub sibling_index: i32,
    #[serde(default, deserialize_with = "null_rect")]
    /// RectTransform geometry.
    pub rect: RectTransform,
    #[serde(default)]
    /// Native v2 final composed screen bbox `[x0, y0, x1, y1]` (1920 reference).
    pub native_rect: Option<[f32; 4]>,
    /// Image/RawImage components.
    #[serde(default, deserialize_with = "null_vec")]
    pub images: Vec<ImageComponent>,
    /// TMP/Text components.
    #[serde(default, deserialize_with = "null_vec")]
    pub texts: Vec<TextComponent>,
    /// CanvasScaler blocks (canvas roots).
    #[serde(default, deserialize_with = "null_vec")]
    pub canvas_scalers: Vec<CanvasScaler>,
    /// Layout groups / layout elements.
    #[serde(default, deserialize_with = "null_vec")]
    pub layouts: Vec<LayoutComponent>,
    /// Button/Canvas/Selectable components.
    #[serde(default, deserialize_with = "null_vec")]
    pub controls: Vec<ControlComponent>,
    /// Child nodes (sibling order).
    #[serde(default, deserialize_with = "null_vec")]
    pub children: Vec<Node>,
}

/// RectTransform serialized values (local to the parent).
#[derive(Clone, Copy, Debug, Deserialize)]
#[allow(dead_code)]
pub struct RectTransform {
    #[serde(default)]
    /// Anchor min.
    pub anchor_min: Fraction,
    #[serde(default)]
    /// Anchor max.
    pub anchor_max: Fraction,
    #[serde(default)]
    /// Anchored position relative to the anchor reference point.
    pub anchored_position: Point,
    #[serde(default)]
    /// Size delta.
    pub size_delta: Point,
    #[serde(default = "default_pivot")]
    /// Pivot in 0..1 rect space.
    pub pivot: Fraction,
    #[serde(default)]
    /// Local position (used by the alternate `Canvas` semantics).
    pub local_position: Point3,
    #[serde(default)]
    /// Local rotation (quaternion).
    pub local_rotation: Quaternion,
    #[serde(default = "default_scale")]
    /// Local scale.
    pub local_scale: Point,
}

impl Default for RectTransform {
    fn default() -> Self {
        Self {
            anchor_min: Fraction::default(),
            anchor_max: Fraction::default(),
            anchored_position: Point::default(),
            size_delta: Point::default(),
            pivot: default_pivot(),
            local_position: Point3::default(),
            local_rotation: Quaternion::default(),
            local_scale: default_scale(),
        }
    }
}

fn default_pivot() -> Fraction {
    Fraction { x: 0.5, y: 0.5 }
}

fn default_scale() -> Point {
    Point { x: 1.0, y: 1.0 }
}

/// A 2D fraction (anchors/pivot).
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct Fraction {
    /// X.
    #[serde(default, deserialize_with = "null_f32")]
    pub x: f32,
    /// Y.
    #[serde(default, deserialize_with = "null_f32")]
    pub y: f32,
}

/// A 2D point/size.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct Point {
    /// X.
    #[serde(default, deserialize_with = "null_f32")]
    pub x: f32,
    /// Y.
    #[serde(default, deserialize_with = "null_f32")]
    pub y: f32,
}

/// A 3D local position.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct Point3 {
    /// X.
    #[serde(default, deserialize_with = "null_f32")]
    pub x: f32,
    /// Y.
    #[serde(default, deserialize_with = "null_f32")]
    pub y: f32,
    /// Z.
    #[serde(default, deserialize_with = "null_f32")]
    pub z: f32,
}

/// A quaternion (rotation is best-effort for UI chrome; most nodes are identity).
#[derive(Clone, Copy, Debug, Deserialize)]
#[allow(dead_code)]
pub struct Quaternion {
    /// X.
    #[serde(default, deserialize_with = "null_f32")]
    pub x: f32,
    /// Y.
    #[serde(default, deserialize_with = "null_f32")]
    pub y: f32,
    /// Z.
    #[serde(default, deserialize_with = "null_f32")]
    pub z: f32,
    /// W.
    #[serde(default, deserialize_with = "null_f32")]
    pub w: f32,
}

impl Default for Quaternion {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        }
    }
}

/// A resolved Image/RawImage component.
#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct ImageComponent {
    #[serde(default, deserialize_with = "null_string")]
    /// Component type (`UnityEngine.UI.Image`, `RawImage`, …).
    pub r#type: String,
    #[serde(default)]
    /// Resolved sprite reference.
    pub sprite: Option<SpriteRef>,
    #[serde(default)]
    /// Runtime color.
    pub color: Option<Color>,
    #[serde(default, deserialize_with = "null_string")]
    /// `Simple` / `Sliced` / `Tiled` / `Filled`.
    pub image_type: String,
    #[serde(default, deserialize_with = "null_string")]
    /// Fill method for `Filled` images.
    pub fill_method: String,
    /// Fill amount (volatile; ignored for layout).
    #[serde(default, deserialize_with = "null_f32")]
    pub fill_amount: f32,
    #[serde(default, deserialize_with = "null_bool")]
    /// Preserve-aspect flag.
    pub preserve_aspect: bool,
    #[serde(default)]
    /// Runtime sprite border (left, bottom, right, top).
    pub runtime_sprite_border: Option<Border>,
    #[serde(default)]
    /// Sprite border as captured in the manifest array order.
    pub border: Option<Vec<f32>>,
    #[serde(default)]
    /// `Sprite.pixelsPerUnit` (the canvas reference is 100).
    pub sprite_ppu: Option<f32>,
    /// The component was disabled in the capture (`Behaviour.enabled == false`): it draws nothing.
    #[serde(default)]
    pub disabled: bool,
}

/// A resolved sprite reference (joined from the manifest/atlas).
#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct SpriteRef {
    #[serde(default, deserialize_with = "null_string")]
    /// Sprite asset name.
    pub name: String,
    #[serde(default)]
    /// Logical UI atlas id (`ui/button/button`).
    pub logical: Option<String>,
    #[serde(default)]
    /// Source asset file.
    pub file: Option<String>,
    #[serde(default, deserialize_with = "null_string")]
    /// `sprite` / `texture` / `material` / `missing`.
    pub kind: String,
    #[serde(default)]
    /// 9-slice border from the atlas (`[left, bottom, right, top]`).
    pub border: Option<Vec<f32>>,
    #[serde(default)]
    /// Default tint from the atlas.
    pub default_tint: Option<[u8; 4]>,
    #[serde(default)]
    /// Joined manifest entry (actual PNG path; beats the aliased logical id).
    pub manifest: Option<SpriteManifest>,
}

/// The manifest entry joined into a runtime sprite (authoritative PNG path).
#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct SpriteManifest {
    #[serde(default, deserialize_with = "null_string")]
    /// Manifest-relative PNG path.
    pub file: String,
}

/// A border serialized as `{x,y,z,w}` (Unity left/bottom/right/top order).
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct Border {
    /// Left.
    #[serde(default, deserialize_with = "null_f32")]
    pub x: f32,
    /// Bottom.
    #[serde(default, deserialize_with = "null_f32")]
    pub y: f32,
    /// Right.
    #[serde(default, deserialize_with = "null_f32")]
    pub z: f32,
    /// Top.
    #[serde(default, deserialize_with = "null_f32")]
    pub w: f32,
}

/// A resolved TMP/Text component.
#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct TextComponent {
    #[serde(default, deserialize_with = "null_string")]
    /// Component type.
    pub r#type: String,
    #[serde(default)]
    /// Captured text (volatile; live values replace known nodes).
    pub text: String,
    #[serde(default)]
    /// Resolved font reference.
    pub font: Option<FontRef>,
    /// Font size in reference pixels.
    #[serde(default, deserialize_with = "null_f32")]
    pub font_size: f32,
    #[serde(default, deserialize_with = "null_string")]
    /// Alignment (`Left`/`Center`/`Right`/…).
    pub alignment: String,
    #[serde(default)]
    /// Text color.
    pub color: Option<Color>,
    #[serde(default, deserialize_with = "null_bool")]
    /// Word wrap flag.
    pub word_wrap: bool,
    #[serde(default, deserialize_with = "null_string")]
    /// Font style (`Bold`/`Italic`/…).
    pub font_style: String,
    #[serde(default, deserialize_with = "null_string")]
    /// TMP text style (`Normal`/`SmallCaps`/`Bold`/…).
    pub text_style: String,
    #[serde(default, deserialize_with = "null_bool")]
    /// Rich text flag.
    pub rich_text: bool,
    #[serde(default, deserialize_with = "null_f32")]
    /// TMP character spacing (font units; 0 in captured sessions).
    pub character_spacing: f32,
    #[serde(default, deserialize_with = "null_f32")]
    /// TMP line spacing (font units; 0 in captured sessions).
    pub line_spacing: f32,
    #[serde(default)]
    /// TMP margins `x` = left, `y` = top, `z` = right, `w` = bottom (reference px).
    pub margin: Option<Border>,
}

/// A resolved font reference.
#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct FontRef {
    #[serde(default, deserialize_with = "null_string")]
    /// TMP font asset name.
    pub name: String,
    #[serde(default)]
    /// Atlas font role (body/bold/mono) when joined.
    pub role: Option<String>,
    #[serde(default)]
    /// Logical font id.
    pub logical: Option<String>,
    #[serde(default, deserialize_with = "null_bool")]
    /// Missing flag (font asset not in the atlas).
    pub missing: bool,
}

/// An RGBA color with float channels.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct Color {
    /// Red 0..1.
    #[serde(default, deserialize_with = "null_f32")]
    pub r: f32,
    /// Green 0..1.
    #[serde(default, deserialize_with = "null_f32")]
    pub g: f32,
    /// Blue 0..1.
    #[serde(default, deserialize_with = "null_f32")]
    pub b: f32,
    #[serde(default = "default_alpha", deserialize_with = "null_f32")]
    pub a: f32,
}

fn default_alpha() -> f32 {
    1.0
}

/// A CanvasScaler block.
#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct CanvasScaler {
    #[serde(default, deserialize_with = "null_string")]
    /// `ConstantPixelSize` / `ScaleWithScreenSize` / `ConstantPhysicalSize`.
    pub ui_scale_mode: String,
    #[serde(default)]
    /// Reference resolution.
    pub reference_resolution: Point,
    #[serde(default, deserialize_with = "null_string")]
    /// Screen match mode.
    pub screen_match_mode: String,
    /// Match width/height (0 = width).
    #[serde(default, deserialize_with = "null_f32")]
    pub match_width_or_height: f32,
    /// Runtime scale factor (volatile).
    #[serde(default, deserialize_with = "null_f32")]
    pub scale_factor: f32,
}

/// A layout group / layout element component (kept for the panel model).
#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct LayoutComponent {
    #[serde(default, deserialize_with = "null_string")]
    /// Component class name.
    pub r#type: String,
    #[serde(default, deserialize_with = "null_bool")]
    /// Enabled flag.
    pub enabled: bool,
}

/// A control component (Button/Canvas/Selectable/…).
#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct ControlComponent {
    #[serde(default, deserialize_with = "null_string")]
    /// Component class name.
    pub r#type: String,
    #[serde(default, deserialize_with = "null_bool")]
    /// Enabled flag.
    pub enabled: bool,
    #[serde(default, deserialize_with = "null_bool")]
    /// Interactable flag (buttons).
    pub interactable: bool,
    #[serde(default)]
    /// Render mode (Canvas).
    pub render_mode: Option<String>,
    /// Toggle state (`UnityEngine.UI.Toggle.is_on`).
    pub is_on: Option<bool>,
    /// Slider current value.
    pub value: Option<f32>,
    /// Slider minimum value.
    pub min_value: Option<f32>,
    /// Slider maximum value.
    pub max_value: Option<f32>,
    /// Slider whole-number mode.
    pub whole_numbers: Option<bool>,
}

/// One core panel identified from `core_panel_paths`.
#[derive(Clone, Debug, PartialEq)]
pub struct CorePanel {
    /// Human name (e.g. `HUD/Controls/TopCenter`, `Right Panels/QuickStatsHolder`).
    pub name: String,
    /// Common path prefix.
    pub path: String,
    /// Number of core paths in this panel.
    pub paths: usize,
    /// Whether any node in the panel is active in the captured hierarchy.
    pub active: bool,
}

/// Loader/coverage statistics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)]
pub struct RuntimeStats {
    /// Total nodes in the session.
    pub nodes: usize,
    /// Nodes active in the captured hierarchy.
    pub active_nodes: usize,
    /// Image components on active nodes.
    pub active_images: usize,
    /// Text components on active nodes.
    pub active_texts: usize,
    /// Draws emitted by the last frame build.
    pub emitted: usize,
}

/// Session wrapper used during deserialization.
#[derive(Deserialize)]
struct AtlasFile {
    #[serde(default)]
    sessions: Sessions,
}

#[derive(Default, Deserialize)]
struct Sessions {
    #[serde(default)]
    default_world: Option<RuntimeSession>,
    #[serde(default)]
    default_world_inspector: Option<RuntimeSession>,
    #[serde(default)]
    default_world_genes: Option<RuntimeSession>,
    #[serde(default)]
    default_world_biology: Option<RuntimeSession>,
    #[serde(default)]
    default_world_brain: Option<RuntimeSession>,
    #[serde(default)]
    main_menu_title: Option<RuntimeSession>,
    #[serde(default)]
    main_menu_settings: Option<RuntimeSession>,
    /// All other captured sessions (functional UI/menu/in-world panels).
    #[serde(flatten)]
    extra: BTreeMap<String, RuntimeSession>,
}

impl RuntimeUi {
    /// Candidate paths for the joined runtime dump.
    #[must_use]
    pub fn candidates() -> Vec<PathBuf> {
        vec![
            PathBuf::from("tools/ui").join(DATA_FILE),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("tools/ui")
                .join(DATA_FILE),
        ]
    }

    /// Finds and loads the runtime dump, returning `Ok(None)` when it is absent.
    pub fn discover() -> Result<Option<Self>, String> {
        for candidate in Self::candidates() {
            if candidate.is_file() {
                return Self::load(&candidate).map(Some);
            }
        }
        Ok(None)
    }

    /// Loads a joined runtime dump (decompressing gzip, with a `target/` cache).
    pub fn load(path: &Path) -> Result<Self, String> {
        let json = read_cached_json(path)?;
        let mut parsed: AtlasFile = serde_json::from_str(&json)
            .map_err(|error| format!("invalid runtime UI JSON: {error}"))?;
        let mut session = parsed
            .sessions
            .default_world
            .take()
            .ok_or_else(|| "runtime UI dump has no `default_world` session".to_owned())?;
        let inspector_session = parsed.sessions.default_world_inspector.take();
        let genes_session = parsed.sessions.default_world_genes.take();
        let biology_session = parsed.sessions.default_world_biology.take();
        let brain_session = parsed.sessions.default_world_brain.take();
        let menu_title = parsed.sessions.main_menu_title.take();
        let menu_settings = parsed.sessions.main_menu_settings.take();
        let extra_sessions = std::mem::take(&mut parsed.sessions.extra);
        let ruler_value = read_ruler_value(path);
        // The capture screen is the reference resolution; fall back to 1920x1080.
        if !session.screen.width.is_finite() || session.screen.width <= 0.0 {
            session.screen.width = REFERENCE_WIDTH;
        }
        if !session.screen.height.is_finite() || session.screen.height <= 0.0 {
            session.screen.height = REFERENCE_HEIGHT;
        }
        Ok(Self {
            semantics: RectSemantics::Local,
            source: path.to_path_buf(),
            session,
            inspector_session,
            genes_session,
            biology_session,
            brain_session,
            menu_title,
            menu_settings,
            extra_sessions,
            lazy_sessions: BTreeMap::new(),
            filtered_menu: std::sync::Mutex::default(),
            dyn_cache: std::sync::Mutex::default(),
            ruler_value,
            native: false,
        })
    }

    /// Copies native named sessions into the fixed fields used by the live
    /// viewer/menu composition, keeping the remaining native sessions available.
    pub fn adopt_native_fixed_sessions(&mut self) {
        let names = [
            ("main_menu_title", 0_u8),
            ("main_menu_settings", 1),
            ("default_world", 2),
            ("default_world_inspector", 3),
            ("default_world_genes", 4),
            ("default_world_biology", 5),
            ("default_world_brain", 6),
        ];
        for (name, slot) in names {
            let Some(session) = self.extra_sessions.remove(name) else {
                continue;
            };
            match slot {
                0 => self.menu_title = Some(session),
                1 => self.menu_settings = Some(session),
                2 => self.session = session,
                3 => self.inspector_session = Some(session),
                4 => self.genes_session = Some(session),
                5 => self.biology_session = Some(session),
                _ => self.brain_session = Some(session),
            }
        }
        self.native = false;
    }

    /// Loads a native 1920 runtime recapture (free-form session names).
    ///
    /// Native dumps have no fixed `default_world` slot; every session is kept in
    /// `extra_sessions` and one becomes the composition default.
    pub fn load_native(path: &Path) -> Result<Self, String> {
        if let Some(split) = Self::load_native_split(path)? {
            return Ok(split);
        }
        let json = read_cached_json(path)?;
        // Per-session raw slices: no whole-document `Value` tree and no clones.
        #[derive(serde::Deserialize)]
        struct Top<'a> {
            #[serde(borrow)]
            sessions: BTreeMap<String, &'a serde_json::value::RawValue>,
        }
        #[derive(serde::Deserialize)]
        struct Canvases {
            #[serde(default)]
            canvases: Vec<serde_json::Value>,
        }
        let top: Top<'_> = serde_json::from_str(&json)
            .map_err(|error| format!("invalid native runtime UI JSON: {error}"))?;
        let mut extra_sessions = BTreeMap::new();
        for (name, raw) in &top.sessions {
            let mut session: RuntimeSession = serde_json::from_str(raw.get())
                .map_err(|error| format!("invalid native session {name}: {error}"))?;
            let canvases: Canvases = serde_json::from_str(raw.get())
                .map_err(|error| format!("invalid native session {name}: {error}"))?;
            session.canvases = canvases
                .canvases
                .iter()
                .map(|canvas| convert_native_node(canvas, true))
                .collect();
            extra_sessions.insert(name.clone(), session);
        }
        let mut session = extra_sessions
            .get("native_world_ui")
            .cloned()
            .or_else(|| extra_sessions.values().next().cloned())
            .ok_or_else(|| "native runtime UI dump has no sessions".to_owned())?;
        if session.screen.width <= 0.0 || session.screen.height <= 0.0 {
            session.screen.width = REFERENCE_WIDTH;
            session.screen.height = REFERENCE_HEIGHT;
        }
        Ok(Self {
            semantics: RectSemantics::Local,
            source: path.to_path_buf(),
            session,
            inspector_session: None,
            genes_session: None,
            biology_session: None,
            brain_session: None,
            menu_title: None,
            menu_settings: None,
            extra_sessions,
            lazy_sessions: BTreeMap::new(),
            filtered_menu: std::sync::Mutex::default(),
            dyn_cache: std::sync::Mutex::default(),
            ruler_value: None,
            native: true,
        })
    }

    /// Per-session files next to the monolith (`ui_runtime_native/<name>.json.gz`,
    /// made by `tools/split_native_runtime.py`): the HUD sessions are parsed now,
    /// every other session on first use (the monolith is ~390 MB of JSON).
    fn load_native_split(path: &Path) -> Result<Option<Self>, String> {
        let Some(parent) = path.parent() else {
            return Ok(None);
        };
        let dir = parent.join("ui_runtime_native");
        let Ok(read) = std::fs::read_dir(&dir) else {
            return Ok(None);
        };
        const EAGER: [&str; 8] = [
            "native_world_ui",
            "main_menu_title",
            "main_menu_settings",
            "default_world",
            "default_world_inspector",
            "default_world_genes",
            "default_world_biology",
            "default_world_brain",
        ];
        let mut extra_sessions = BTreeMap::new();
        let mut lazy_sessions = BTreeMap::new();
        for entry in read.flatten() {
            let file = entry.path();
            let Some(name) = file
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".json.gz"))
                .map(str::to_owned)
            else {
                continue;
            };
            if EAGER.contains(&name.as_str()) {
                let session = load_session_file(&file)?;
                extra_sessions.insert(name, session);
            } else {
                lazy_sessions.insert(name, (file, std::sync::OnceLock::new()));
            }
        }
        if extra_sessions.is_empty() && lazy_sessions.is_empty() {
            return Ok(None);
        }
        let mut session = extra_sessions
            .get("native_world_ui")
            .cloned()
            .or_else(|| extra_sessions.values().next().cloned())
            .ok_or_else(|| "native runtime UI dump has no sessions".to_owned())?;
        if session.screen.width <= 0.0 || session.screen.height <= 0.0 {
            session.screen.width = REFERENCE_WIDTH;
            session.screen.height = REFERENCE_HEIGHT;
        }
        Ok(Some(Self {
            semantics: RectSemantics::Local,
            source: path.to_path_buf(),
            session,
            inspector_session: None,
            genes_session: None,
            biology_session: None,
            brain_session: None,
            menu_title: None,
            menu_settings: None,
            extra_sessions,
            lazy_sessions,
            filtered_menu: std::sync::Mutex::default(),
            dyn_cache: std::sync::Mutex::default(),
            ruler_value: None,
            native: true,
        }))
    }

    /// Parses an in-memory JSON document (tests/experiments).
    #[allow(dead_code)]
    pub fn from_json(json: &str) -> Result<Self, String> {
        let mut parsed: AtlasFile = serde_json::from_str(json)
            .map_err(|error| format!("invalid runtime UI JSON: {error}"))?;
        let session = parsed
            .sessions
            .default_world
            .take()
            .ok_or_else(|| "runtime UI dump has no `default_world` session".to_owned())?;
        Ok(Self {
            semantics: RectSemantics::Local,
            source: PathBuf::from("<memory>"),
            session,
            inspector_session: None,
            genes_session: None,
            biology_session: None,
            brain_session: None,
            menu_title: None,
            menu_settings: None,
            extra_sessions: BTreeMap::new(),
            lazy_sessions: BTreeMap::new(),
            filtered_menu: std::sync::Mutex::default(),
            dyn_cache: std::sync::Mutex::default(),
            ruler_value: None,
            native: false,
        })
    }

    /// Loader/coverage statistics (total + active node/component counts).
    #[must_use]
    pub fn stats(&self) -> RuntimeStats {
        let mut stats = RuntimeStats {
            nodes: 0,
            active_nodes: 0,
            active_images: 0,
            active_texts: 0,
            emitted: 0,
        };
        for node in &self.session.canvases {
            count_nodes(node, &mut stats);
        }
        stats
    }

    /// Decodes the active nodes' runtime sprites that are not in the UI atlas
    /// (the joined dump references a few PNGs the atlas generator skipped).
    ///
    /// Returns the number of newly decoded textures.
    pub fn load_sprite_textures(&self, ui: &mut UiAssets) -> Result<usize, String> {
        let mut files = Vec::new();
        for session in self.sessions() {
            for root in active_canvas_roots(session) {
                collect_sprite_files(root, &mut files);
            }
        }
        files.sort();
        files.dedup();
        let mut loaded = 0;
        for file in files {
            if ui.texture_of_file(&file).is_none() {
                ui.add_texture_file(&file)?;
                loaded += 1;
            }
        }
        Ok(loaded)
    }

    /// The pointer zones of the three bottom-left toolbar buttons in screen pixels: the button,
    /// plus its panel while that is open (`OnHoverShow`: the panel is a child of the button, so
    /// resting on either keeps it open).
    #[must_use]
    pub fn toolbar_zones(&self, app: &App, width: f32, height: f32) -> [Option<(Vec2, Vec2)>; 3] {
        let scale = UiFrame::scaler(width, height);
        let mut zones = [None; 3];
        for (index, (session_name, _, button, panel)) in TOOLBAR.iter().enumerate() {
            let Some(button) = compose_path(&self.session, button) else {
                continue;
            };
            let (mut min, mut max) = (button.pos, button.pos + button.size);
            if app.toolbar[index].open
                && let Some(session) = self.session_by_name(session_name)
                && let Some(panel) = compose_path(session, panel)
            {
                min = min.min(panel.pos);
                max = max.max(panel.pos + panel.size);
            }
            zones[index] = Some((min * scale, (max - min) * scale));
        }
        zones
    }

    /// Builds the core panel model from `core_panel_paths` (HUD, playback,
    /// right-side panel stack, bibite buttons). Used for coverage reporting and
    /// tests; frame drawing walks the hierarchy directly.
    #[must_use]
    pub fn core_panels(&self) -> Vec<CorePanel> {
        use std::collections::BTreeMap;
        let mut panels: BTreeMap<String, (usize, bool)> = BTreeMap::new();
        let active_by_prefix = self.active_path_set();
        for path in &self.session.core_panel_paths {
            let (name, prefix) = panel_key(path);
            let entry = panels.entry(name).or_insert((0, false));
            entry.0 += 1;
            if active_by_prefix.iter().any(|active| {
                *active == prefix
                    || active.starts_with(&format!("{prefix}/"))
                    || prefix.starts_with(active)
            }) {
                entry.1 = true;
            }
        }
        panels
            .into_iter()
            .map(|(name, (paths, active))| CorePanel {
                path: name.clone(),
                name,
                paths,
                active,
            })
            .collect()
    }

    fn active_path_set(&self) -> Vec<String> {
        let mut paths = Vec::new();
        for canvas in &self.session.canvases {
            collect_active_paths(canvas, &mut paths);
        }
        paths
    }

    /// Builds a screen-space UI frame from the active runtime nodes.
    ///
    /// Draw positions/sizes are in the captured 1920x1080 reference space and
    /// scaled by [`UiFrame::scaler`] (same as the curated HUD). Runtime button
    /// rects are registered as hit rectangles with mapped [`UiAction`]s so the
    /// existing `--click`/interaction path works on the runtime HUD.
    #[cfg(test)]
    #[must_use]
    pub fn build_frame(&self, app: &App, ui: &UiAssets, width: f32, height: f32) -> UiFrame {
        self.build_frame_with_fonts(app, ui, None, width, height)
    }

    /// [`Self::build_frame`] with the fonts the Stats panel's layout measures its texts with (without them it estimates).
    pub fn build_frame_with_fonts(
        &self,
        app: &App,
        ui: &UiAssets,
        fonts: Option<&crate::text_sdf::SdfFonts>,
        width: f32,
        height: f32,
    ) -> UiFrame {
        // `GUIManager.curMenu`: a selected bibite shows the tab buttons and the status bars; a
        // panel opens only once one is chosen. Without a panel the base HUD gets just those two
        // groups of the inspector capture (below).
        let session_name = if app.inspector_visible() {
            match app.inspector_tab {
                InspectorTab::Stats => "default_world_inspector",
                InspectorTab::Genes => "default_world_genes",
                InspectorTab::Biology => "default_world_biology",
                InspectorTab::Brain => "default_world_brain",
            }
        } else {
            "default_world"
        };
        let session = self.frame_session(app);
        // Native hit ids start high so they never collide with the curated
        // widgets (100..8999) or the overlay frames (20000..) drawn on top.
        let mut frame = self.build_session_frame_filtered(
            session_name,
            session,
            Some(app),
            fonts,
            ui,
            width,
            height,
            None,
            30_000,
            None,
        );
        if app.selection.is_some()
            && !app.inspector_visible()
            && let (Some(only), Some(inspector)) = (
                self.overlay_paths_under(
                    "default_world_inspector",
                    &["/statusbars", "/Panels/BibiteButtons"],
                ),
                self.session_by_name("default_world_inspector"),
            )
        {
            // Ids sit between the toolbar overlays (20000..) and the base frame (30000..).
            let extra = self.build_session_frame_filtered(
                "default_world_inspector",
                inspector,
                Some(app),
                fonts,
                ui,
                width,
                height,
                Some(&only),
                26_000,
                None,
            );
            frame.draws.extend(extra.draws);
            frame.hits.extend(extra.hits);
            frame.actions.extend(extra.actions);
            frame.tips.extend(extra.tips);
        }
        // Panels opened from the toolbar / top-right buttons replay the nodes the
        // game activates for them (captured as separate sessions).
        // (session, panel root): the captures are cumulative click-through
        // states, so only the nodes under the panel's own root are replayed.
        let mut overlays: Vec<(&str, &str)> = Vec::new();
        for (index, (session, root, _, _)) in TOOLBAR.iter().enumerate() {
            if app.toolbar[index].open {
                overlays.push((session, root));
            }
        }
        if app.lineage_gate_popup {
            overlays.push(("main_world_lineage", "/TopLayer/Popuholder"));
        }
        for (index, (name, root)) in overlays.into_iter().enumerate() {
            let (Some(only), Some(panel_session)) =
                (self.overlay_paths(name, root), self.session_by_name(name))
            else {
                continue;
            };
            let extra = self.build_session_frame_filtered(
                name,
                panel_session,
                Some(app),
                fonts,
                ui,
                width,
                height,
                Some(&only),
                20_000 + 1_000 * index as UiId,
                None,
            );
            frame.draws.extend(extra.draws);
            frame.hits.extend(extra.hits);
            frame.actions.extend(extra.actions);
            frame.tips.extend(extra.tips);
        }
        if app.panels.is_open(PanelKind::WorldDynamics)
            && let Some(extra) = self.build_dyn_panel_frame(app, ui, fonts, width, height)
        {
            frame.draws.extend(extra.draws);
            frame.hits.extend(extra.hits);
            frame.actions.extend(extra.actions);
            frame.tips.extend(extra.tips);
        }
        frame
    }

    /// The dynamic settings panel: the captured chrome and row prefabs with live rows
    /// (`dyn_panel`), replayed through the normal node pipeline.
    fn build_dyn_panel_frame(
        &self,
        app: &App,
        ui: &UiAssets,
        fonts: Option<&crate::text_sdf::SdfFonts>,
        width: f32,
        height: f32,
    ) -> Option<UiFrame> {
        fn is_panel(node: &Node) -> bool {
            node.path.ends_with("/Right Panels/DynamicSettingsPanel")
        }
        fn find_panel(node: &Node) -> Option<&Node> {
            if is_panel(node) {
                return Some(node);
            }
            node.children.iter().find_map(find_panel)
        }
        /// The chain of ancestors down to the panel, with the panel replaced.
        fn chain_with(node: &Node, replacement: &Node) -> Option<Node> {
            if is_panel(node) {
                return Some(replacement.clone());
            }
            for child in &node.children {
                if let Some(rebuilt) = chain_with(child, replacement) {
                    // Only the ancestor itself is cloned, not its other children.
                    let mut copy = crate::dyn_panel::shallow(node);
                    copy.children = vec![rebuilt];
                    return Some(copy);
                }
            }
            None
        }
        fn collect(node: &Node, out: &mut HashSet<String>) {
            out.insert(node.path.clone());
            for child in &node.children {
                collect(child, out);
            }
        }
        let session = self.session_by_name("main_world_dynamic_parameters")?;
        let template = session.canvases.iter().find_map(find_panel)?;
        // The default bibites list: the live spawn rows, with the zones they can be limited to.
        let names = app.default_bibite_names();
        let zones: Vec<bibites_core::settings::ZoneSettings> = app
            .world
            .zones
            .zones
            .iter()
            .map(|zone| zone.config.0.clone())
            .collect();
        // The Zones list holds the explicit zones (`ScenarioSettings.zones`); the groups' zones belong to their groups.
        let explicit = app.world.zones.explicit_count();
        // The settings an animator could take over (the Animation tab's base items name them).
        let catalog = if app.dyn_panel.tab == 4 { app.animator_catalog() } else { Vec::new() };
        let input = crate::dyn_panel::DynInput {
            settings: &app.world.settings,
            state: &app.dyn_panel,
            tps_defaults: app.tps_defaults,
            edit: app.value_edit.as_ref(),
            lists: Some(crate::dyn_panel::ListsInput {
                zones: &zones[..explicit],
                all_zones: &zones,
                changers: &app.world.settings_changers,
                catalog: &catalog,
                graph_views: &app.animator.views,
                sim_time: Some(app.world.simulated_time as f32),
                selected_point: app.animator.point,
                creator: Some(&app.animator.creator),
                groups: &app.world.zone_groups,
                open: &app.dyn_panel.open_items,
                order: &app.dyn_panel.order,
                bibites: &app.world.bibite_spawns,
                bibite_names: &names,
                in_sim: true,
                delete_item: |list, index| {
                    UiAction::Invoke(match list {
                        0 => ControlCommand::DeleteZone(index),
                        1 => ControlCommand::DeleteZoneGroup(index),
                        _ => ControlCommand::DeleteDefaultBibite(index),
                    })
                },
                sim_size: app.world.settings.independents.simulation_size,
                name_edit: app
                    .zone_name_edit
                    .as_ref()
                    .map(|(index, text)| (*index as u16, text.as_str())),
            }),
        };
        // The list header and heading prefabs come from the scenario editor's capture (the
        // world panel was captured on its Simulation tab).
        let editor = self
            .session_by_name("main_menu_scenario_editor")
            .and_then(|editor| {
                fn find_editor(node: &Node) -> Option<&Node> {
                    if node.path == "UI/ScenarioEditor" {
                        return Some(node);
                    }
                    node.children.iter().find_map(find_editor)
                }
                editor.canvases.iter().find_map(find_editor)
            });
        let key = crate::dyn_panel::state_key(&input)
            ^ u64::from(width.to_bits()).rotate_left(17)
            ^ u64::from(height.to_bits());
        if let Ok(cache) = self.dyn_cache.lock()
            && let Some((cached_key, cached)) = cache.as_ref()
            && *cached_key == key
        {
            return Some(cached.clone());
        }
        let build = crate::dyn_panel::build_with_extras(template, editor, &input)?;
        let mut only = HashSet::new();
        collect(&build.root, &mut only);
        let chain = session
            .canvases
            .iter()
            .find_map(|canvas| chain_with(canvas, &build.root))?;
        let synthetic = RuntimeSession {
            screen: session.screen,
            canvases: vec![chain],
            core_panel_paths: Vec::new(),
            core_panel_count: 0,
            node_count: 0,
        };
        let mut frame = self.build_session_frame_filtered(
            "dyn_panel",
            &synthetic,
            Some(app),
            fonts,
            ui,
            width,
            height,
            Some(&only),
            22_000,
            None,
        );
        let scale = UiFrame::scaler(width, height);
        // The click areas are laid out for the capture's 1920 x 1080 canvas; the panel (its nodes, drawn above) is flush with the right edge of
        // the canvas there is: they move with it, `dx` reference px right (the sliders' tracks they carry too).
        let dx = dyn_panel_offset(Vec2::new(width / scale, height / scale));
        let [px0, py0, px1, py1] = crate::dyn_panel::PANEL;
        // The panel swallows clicks meant for the world underneath.
        frame.hit(
            21_900,
            Vec2::new((px0 + dx) * scale, py0 * scale),
            Vec2::new((px1 - px0) * scale, (py1 - py0) * scale),
            UiAction::Consume,
        );
        for (index, hit) in build.hits.iter().enumerate() {
            frame.hit(
                22_500 + index as UiId,
                Vec2::new((hit.rect[0] + dx) * scale, hit.rect[1] * scale),
                Vec2::new(hit.rect[2] * scale, hit.rect[3] * scale),
                crate::dyn_panel::offset_action(hit.action, dx),
            );
        }
        // The rows' name labels show the settings' help texts.
        for tip in &build.tips {
            frame.tips.push(crate::tooltip::TipRect {
                pos: Vec2::new((tip.rect[0] + dx) * scale, tip.rect[1] * scale),
                size: Vec2::new(tip.rect[2] * scale, tip.rect[3] * scale),
                tip: tip.tip,
            });
        }
        if let Ok(mut cache) = self.dyn_cache.lock() {
            *cache = Some((key, frame.clone()));
        }
        Some(frame)
    }

    /// The scenario editor: the captured chrome and row prefabs with the draft's live rows
    /// (`scenario_editor`), replayed through the normal node pipeline, plus its dialogs.
    fn build_scenario_editor_frame(
        &self,
        menu: &crate::menu::MenuState,
        ui: &UiAssets,
        width: f32,
        height: f32,
        metrics: Option<&crate::text_sdf::SdfFonts>,
    ) -> Option<UiFrame> {
        fn is_editor(node: &Node) -> bool {
            node.path == "UI/ScenarioEditor"
        }
        fn find_editor(node: &Node) -> Option<&Node> {
            if is_editor(node) {
                return Some(node);
            }
            node.children.iter().find_map(find_editor)
        }
        fn chain_with(node: &Node, replacement: &Node) -> Option<Node> {
            if is_editor(node) {
                return Some(replacement.clone());
            }
            for child in &node.children {
                if let Some(rebuilt) = chain_with(child, replacement) {
                    let mut copy = crate::dyn_panel::shallow(node);
                    copy.children = vec![rebuilt];
                    return Some(copy);
                }
            }
            None
        }
        fn collect(node: &Node, out: &mut HashSet<String>) {
            out.insert(node.path.clone());
            for child in &node.children {
                collect(child, out);
            }
        }
        let editor = menu.editor()?;
        let session = self.session_by_name("main_menu_scenario_editor")?;
        let captured = session.canvases.iter().find_map(find_editor)?;
        let tps = menu.tps_defaults();
        let scale = UiFrame::scaler(width, height);
        let key = crate::scenario_editor::state_key(editor, tps)
            ^ u64::from(width.to_bits()).rotate_left(17)
            ^ u64::from(height.to_bits());
        let cached = self.dyn_cache.lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|(k, _)| *k == key)
                .map(|(_, f)| f.clone())
        });
        let mut frame = if let Some(frame) = cached {
            frame
        } else {
            let measure = |text: &str, px: f32| -> f32 {
                metrics.map_or(text.chars().count() as f32 * px * 0.5, |metrics| {
                    metrics.measure_named(
                        Some("Roboto-Bold SDF NoOutline"),
                        FontRole::Bold,
                        text,
                        px,
                    )
                })
            };
            let library: Vec<(String, String)> = menu
                .templates()
                .iter()
                .filter_map(|entry| {
                    let file = entry.path.file_name()?.to_str()?.to_owned();
                    Some((file, entry.name.clone()))
                })
                .collect();
            let input = crate::scenario_editor::EditorInput {
                state: editor,
                tps_defaults: tps,
                measure: &measure,
                library: &library,
            };
            let build = crate::scenario_editor::build(captured, &input)?;
            let mut only = HashSet::new();
            collect(&build.root, &mut only);
            let chain = session
                .canvases
                .iter()
                .find_map(|canvas| chain_with(canvas, &build.root))?;
            let synthetic = RuntimeSession {
                screen: session.screen,
                canvases: vec![chain],
                core_panel_paths: Vec::new(),
                core_panel_count: 0,
                node_count: 0,
            };
            let mut frame = self.build_session_frame_filtered(
                "scenario_editor",
                &synthetic,
                None,
                metrics,
                ui,
                width,
                height,
                Some(&only),
                22_000,
                None,
            );
            for (index, hit) in build.hits.iter().enumerate() {
                frame.hit(
                    22_500 + index as UiId,
                    Vec2::new(hit.rect[0] * scale, hit.rect[1] * scale),
                    Vec2::new(hit.rect[2] * scale, hit.rect[3] * scale),
                    hit.action,
                );
            }
            if let Ok(mut cache) = self.dyn_cache.lock() {
                *cache = Some((key, frame.clone()));
            }
            frame
        };
        if !menu.status().is_empty() {
            frame.text(
                menu.status(),
                Vec2::new(24.0 * scale, (height / scale - 34.0) * scale),
                20.0 * scale,
                [255, 225, 140, 255],
            );
        }
        emit_editor_zone_preview(
            &mut frame,
            &crate::scenario_editor::preview_shapes(&editor.settings, &editor.scenario),
            menu.user_settings().color("Fertility Color"),
            scale,
        );
        if let Some(popup) = editor.popup.as_ref() {
            self.emit_editor_popup(&mut frame, menu, popup, ui, width, height, metrics);
        }
        Some(frame)
    }

    /// Replays `replacement` in place of the node matching `is_target` of a captured session
    /// (its ancestors are kept as empty shells so the layout and canvas scale stay right).
    #[allow(clippy::too_many_arguments)]
    fn replay_replacement(
        &self,
        session_name: &str,
        is_target: &dyn Fn(&Node) -> bool,
        replacement: &Node,
        ui: &UiAssets,
        width: f32,
        height: f32,
        metrics: Option<&crate::text_sdf::SdfFonts>,
        first_hit: UiId,
    ) -> Option<UiFrame> {
        fn chain_with(
            node: &Node,
            is_target: &dyn Fn(&Node) -> bool,
            replacement: &Node,
        ) -> Option<Node> {
            if is_target(node) {
                return Some(replacement.clone());
            }
            for child in &node.children {
                if let Some(rebuilt) = chain_with(child, is_target, replacement) {
                    let mut copy = crate::dyn_panel::shallow(node);
                    copy.children = vec![rebuilt];
                    return Some(copy);
                }
            }
            None
        }
        fn collect(node: &Node, out: &mut HashSet<String>) {
            out.insert(node.path.clone());
            for child in &node.children {
                collect(child, out);
            }
        }
        let session = self.session_by_name(session_name)?;
        let chain = session
            .canvases
            .iter()
            .find_map(|canvas| chain_with(canvas, is_target, replacement))?;
        let mut only = HashSet::new();
        collect(replacement, &mut only);
        let synthetic = RuntimeSession {
            screen: session.screen,
            canvases: vec![chain],
            core_panel_paths: Vec::new(),
            core_panel_count: 0,
            node_count: 0,
        };
        Some(self.build_session_frame_filtered(
            "scenario_editor",
            &synthetic,
            None,
            metrics,
            ui,
            width,
            height,
            Some(&only),
            first_hit,
            None,
        ))
    }

    /// The in-world confirmation dialog (`PopupManager.DisplayChoiceDialog`) in the game's popup
    /// look, with the game's own title, text and button labels, drawn over everything else.
    pub fn emit_confirm_dialog(
        &self,
        frame: &mut UiFrame,
        app: &App,
        ui: &UiAssets,
        metrics: Option<&crate::text_sdf::SdfFonts>,
        width: f32,
        height: f32,
    ) {
        let Some(kind) = app.confirm_dialog.as_ref() else {
            return;
        };
        // `DisplayActionWarning`: the warning popup, with its "do not show again" checkbox, over
        // the screen blocker. The world's `Popuholder` scales it by 1.35.
        if let Some(warning) = kind.warning() {
            crate::warning_popup::emit(
                frame,
                ui,
                metrics,
                width,
                height,
                &crate::warning_popup::Spec {
                    title: warning.shown_title(),
                    text: warning.text(),
                    proceed: warning.proceed_label(),
                    checked: app.confirm_do_not_show,
                    scale: 1.35,
                },
                crate::warning_popup::Actions {
                    cancel: UiAction::Invoke(ControlCommand::Cancel),
                    proceed: UiAction::Invoke(ControlCommand::Confirm),
                    toggle: UiAction::Invoke(ControlCommand::ToggleDoNotShowAgain),
                },
                44_300,
            );
            return;
        }
        // The zone or animator being deleted is named in the question (`settings.zoneName.val`, `changerName.val`).
        let zone_name = match kind {
            crate::app::ConfirmKind::DeleteZone(index) => app
                .world
                .zones
                .zones
                .get(*index)
                .map_or_else(String::new, |zone| zone.config.0.name.clone()),
            crate::app::ConfirmKind::DeleteAnimator(index) => app
                .world
                .settings_changers
                .get(*index)
                .map_or_else(String::new, |changer| changer.name.clone()),
            _ => String::new(),
        };
        let (title, text, buttons) = confirm_texts(kind, &zone_name);
        let Some((dialog, rects)) = self.dialog_frame(
            ui, width, height, metrics, title, &text, &buttons, 1.35, 44_100,
        ) else {
            return;
        };
        let scale = UiFrame::scaler(width, height);
        frame.draws.extend(dialog.draws);
        // The dismiss button is first, the accept button second.
        for (index, rect) in rects.iter().enumerate() {
            let command = if index == 0 {
                ControlCommand::Cancel
            } else {
                ControlCommand::Confirm
            };
            frame.hit(
                44_200 + index as UiId,
                Vec2::new(rect[0] * scale, rect[1] * scale),
                Vec2::new(rect[2] * scale, rect[3] * scale),
                UiAction::Invoke(command),
            );
        }
    }

    /// A modal `DialogPopup` in the game's look: title, wrapped text and buttons, centred on
    /// the screen. `popup_scale` is the scene's `Popuholder` scale: 1.35 in the world (the 400-unit
    /// panel is 540 px wide, as measured on the captured popup), 1.0 in the title menu. Returns the
    /// frame and the button rectangles (`x, y, w, h`, reference px).
    #[allow(clippy::too_many_arguments)]
    fn dialog_frame(
        &self,
        ui: &UiAssets,
        width: f32,
        height: f32,
        metrics: Option<&crate::text_sdf::SdfFonts>,
        title: &str,
        text: &str,
        buttons: &[&str],
        popup_scale: f32,
        first_hit: UiId,
    ) -> Option<(UiFrame, Vec<[f32; 4]>)> {
        use crate::dyn_panel::{find, find_mut, set_rect, set_text};
        /// The `Popuholder` scale the template was captured under.
        const CAPTURED_SCALE: f32 = 1.35;
        // The prefab's layout in canvas units, times the holder scale: a 400-wide panel (padding 10 /
        // 10 / 5 / 10, spacing 10) with a 28-high title, the wrapped text (18 pt Liberation Sans, a
        // 20.41-unit line as measured) and a 30-high row of 80-wide buttons.
        let k = popup_scale;
        let left = 960.0 - 200.0 * k;
        let inner = (960.0 - 190.0 * k, 960.0 + 190.0 * k);
        let line_height = 27.55 / CAPTURED_SCALE * k;
        let button = (80.0 * k, 30.0 * k);
        let session = self.session_by_name("lineage_species_panel")?;
        let is_dialog =
            |n: &Node| n.path.ends_with("/Popuholder/DialogPopup(Clone)") && n.active_in_hierarchy;
        let template = session
            .canvases
            .iter()
            .find_map(|canvas| find(canvas, &is_dialog))?;
        let mut dialog = template.clone();
        // The replay multiplies the font sizes by the captured holder's scale.
        fn scale_fonts(node: &mut Node, factor: f32) {
            for text in &mut node.texts {
                text.font_size *= factor;
            }
            for child in &mut node.children {
                scale_fonts(child, factor);
            }
        }
        scale_fonts(&mut dialog, k / CAPTURED_SCALE);
        let font_px = 18.0 * k;
        let measure = |line: &str| -> f32 {
            metrics.map_or(line.chars().count() as f32 * font_px * 0.5, |metrics| {
                metrics.measure_named(Some("LiberationSans SDF"), FontRole::Body, line, font_px)
            })
        };
        let wrapped = wrap_words(text, inner.1 - inner.0, &measure);
        let line_count = wrapped.split('\n').count().max(1);
        let text_height = line_count as f32 * line_height;
        let panel_height = 93.0 * k + text_height;
        let top = 540.0 - panel_height * 0.5;
        set_rect(&mut dialog, left, top, left + 400.0 * k, top + panel_height);
        let title_top = top + 5.0 * k;
        let text_top = title_top + 28.0 * k + 10.0 * k;
        let holder_top = text_top + text_height + 10.0 * k;
        if let Some(node) = find_mut(&mut dialog, &|n: &Node| n.name == "DialogTitle") {
            set_rect(node, inner.0, title_top, inner.1, title_top + 28.0 * k);
            set_text(node, title);
        }
        if let Some(node) = find_mut(&mut dialog, &|n: &Node| n.name == "DialogText") {
            set_rect(node, inner.0, text_top, inner.1, text_top + text_height);
            set_text(node, &wrapped);
        }
        let mut rects = Vec::new();
        if let Some(holder) = find_mut(&mut dialog, &|n: &Node| n.name == "buttonHolder") {
            set_rect(holder, inner.0, holder_top, inner.1, holder_top + button.1);
            let Some(template_button) = holder.children.first().cloned() else {
                return None;
            };
            holder.children.clear();
            for (index, label) in buttons.iter().enumerate() {
                let mut node = template_button.clone();
                // First button left, the others right (the holder's spacer pushes them apart).
                let x0 = if buttons.len() > 1 && index == 0 {
                    inner.0
                } else {
                    inner.1 - button.0
                };
                set_rect(
                    &mut node,
                    x0,
                    holder_top,
                    x0 + button.0,
                    holder_top + button.1,
                );
                node.sibling_index = index as i32;
                if let Some(label_node) = find_mut(&mut node, &|n: &Node| !n.texts.is_empty()) {
                    set_rect(
                        label_node,
                        x0,
                        holder_top,
                        x0 + button.0,
                        holder_top + button.1,
                    );
                    set_text(label_node, label);
                }
                rects.push([x0, holder_top, button.0, button.1]);
                holder.children.push(node);
            }
        }
        let frame = self.replay_replacement(
            "lineage_species_panel",
            &is_dialog,
            &dialog,
            ui,
            width,
            height,
            metrics,
            first_hit,
        )?;
        Some((frame, rects))
    }

    /// The editor's dialogs (exit / start / overwrite warnings and the save form).
    #[allow(clippy::too_many_arguments)]
    fn emit_editor_popup(
        &self,
        frame: &mut UiFrame,
        menu: &crate::menu::MenuState,
        popup: &crate::scenario_editor::EditorPopup,
        ui: &UiAssets,
        width: f32,
        height: f32,
        metrics: Option<&crate::text_sdf::SdfFonts>,
    ) {
        use crate::scenario_editor::EditorPopup;
        let scale = UiFrame::scaler(width, height);
        // `ScenariosEditor.StartSimulation`: an action warning with its "do not show again" box.
        if matches!(popup, EditorPopup::StartWarning) {
            let warning = crate::warnings::ActionWarning::StartScenarioAfterEdit;
            crate::warning_popup::emit(
                frame,
                ui,
                metrics,
                width,
                height,
                &crate::warning_popup::Spec {
                    title: warning.shown_title(),
                    text: warning.text(),
                    proceed: warning.proceed_label(),
                    checked: menu.confirm_checked(),
                    scale: 1.0,
                },
                crate::warning_popup::Actions {
                    cancel: UiAction::Menu(crate::ui::MenuCommand::EditorAnswer(false)),
                    proceed: UiAction::Menu(crate::ui::MenuCommand::EditorAnswer(true)),
                    toggle: UiAction::Menu(crate::ui::MenuCommand::ToggleDoNotShowAgain),
                },
                43_300,
            );
            return;
        }
        // The dialogs dim and block everything behind them.
        frame.rect(Vec2::ZERO, Vec2::new(width, height), [0, 0, 0, 150]);
        frame.hit(
            43_000,
            Vec2::ZERO,
            Vec2::new(width, height),
            UiAction::Consume,
        );
        let mut dialog_text: Option<(String, String)> = None;
        let (title, text, buttons): (&str, &str, [&str; 2]) = match popup {
            EditorPopup::ExitWarning => (
                "Warning",
                "You're about to exit the Scenario Editor without saving it in a new scenario or starting your simulation.\nYou will lose any modifications, and values will be reset to their default.\nAre you sure?",
                ["Cancel", "YES"],
            ),
            EditorPopup::StartWarning => (
                "Launching Simulation",
                "You're about to launch the Simulation from a modified Scenario. Make sure to save the scenario first if you intend to use it again later.\nIt's also possible to save the scenario once in-game.\n\nAre you sure you are Ready",
                ["Cancel", "YES"],
            ),
            EditorPopup::Overwrite => (
                "Warning",
                "A scenario with this name already exists.\nContinuing will overwrite the previous scenario with this one.\nAre you sure?",
                ["Cancel", "YES"],
            ),
            EditorPopup::DeleteItem(list, index) => {
                let name = menu
                    .editor()
                    .map(|editor| editor.item_name(*list, usize::from(*index)))
                    .unwrap_or_default();
                dialog_text = Some((
                    match *list {
                        0 => "Delete Zone",
                        3 => "Delete Settings Animator",
                        _ => "Delete Zone Group",
                    }
                    .to_owned(),
                    format!("Are you sure you want to delete {name}?\nYou will lose all data"),
                ));
                ("", "", ["Cancel", "YES"])
            }
            EditorPopup::ClearList(list) => match list {
                0 => (
                    "Delete All Zones",
                    "Are you sure you want to delete all zones? You will lose all data",
                    ["Cancel", "YES"],
                ),
                1 => (
                    "Delete All Zone Groups",
                    "Are you sure you want to delete all zone groups? You will lose all data",
                    ["Cancel", "YES"],
                ),
                2 => (
                    "Delete All Default Bibites",
                    "Are you sure you want to delete all default bibites? You will lose all data.",
                    ["Cancel", "YES"],
                ),
                _ => (
                    "Delete All Settings Animators",
                    "Are you sure you want to delete all settings animators? You will lose all data",
                    ["Cancel", "YES"],
                ),
            },
            EditorPopup::Save { .. } => {
                self.emit_save_form(frame, menu, ui, width, height, metrics);
                return;
            }
        };
        let (title, text) = dialog_text.as_ref().map_or((title, text), |(title, text)| {
            (title.as_str(), text.as_str())
        });
        let Some((dialog, rects)) = self.dialog_frame(
            ui, width, height, metrics, title, text, &buttons, 1.0, 43_100,
        ) else {
            return;
        };
        frame.draws.extend(dialog.draws);
        for (index, rect) in rects.iter().enumerate() {
            frame.hit(
                43_200 + index as UiId,
                Vec2::new(rect[0] * scale, rect[1] * scale),
                Vec2::new(rect[2] * scale, rect[3] * scale),
                UiAction::Menu(crate::ui::MenuCommand::EditorAnswer(index == 1)),
            );
        }
    }

    /// The save form (`ScenarioSaver`): the captured `Saver` panel (its layout was captured
    /// while hidden, so every rectangle is placed here from the measured anatomy) with the typed
    /// name and description, the preview placeholder and the save / back buttons.
    fn emit_save_form(
        &self,
        frame: &mut UiFrame,
        menu: &crate::menu::MenuState,
        ui: &UiAssets,
        width: f32,
        height: f32,
        metrics: Option<&crate::text_sdf::SdfFonts>,
    ) {
        use crate::dyn_panel::{find, find_mut, set_active, set_rect, set_text};
        use crate::scenario_editor::EditorPopup;
        let scale = UiFrame::scaler(width, height);
        let Some(editor) = menu.editor() else {
            return;
        };
        let focus = match editor.popup {
            Some(EditorPopup::Save { focus }) => focus,
            _ => 0,
        };
        let Some(session) = self.session_by_name("main_menu_scenario_editor") else {
            return;
        };
        let is_saver = |n: &Node| n.path == "UI/ScenarioEditor/Saver";
        let Some(template) = session.canvases.iter().find_map(|c| find(c, &is_saver)) else {
            return;
        };
        let mut root = template.clone();
        set_active(&mut root, true);
        root.images.clear();
        // Panel anatomy (reference px, y down): the 1100 x 558 container is centred; the button
        // row is 70 px tall, then the 478 px framed panel with the two columns inside.
        let (px, py) = (410.0_f32, 239.5_f32);
        let (panel_top, panel_bottom) = (py + 80.0, py + 558.1);
        let (details_x, details_w) = (px + 25.0, 600.0);
        let (preview_x, content_top) = (px + 675.0, panel_top + 25.0);
        let place = |root: &mut Node, suffix: &str, rect: [f32; 4]| {
            if let Some(node) = find_mut(root, &|n: &Node| n.path.ends_with(suffix)) {
                set_rect(node, rect[0], rect[1], rect[2], rect[3]);
            }
        };
        let valid = menu.editor_name_problem().is_none();
        place(
            &mut root,
            "/button holder/ Buttons/SaveSettings",
            [px, py, px + 70.0, py + 70.0],
        );
        place(
            &mut root,
            "/button holder/ Buttons/BackButton",
            [px + 1030.0, py, px + 1100.0, py + 70.0],
        );
        place(
            &mut root,
            "/GameObject/SelectedPanel",
            [px, panel_top, px + 1100.0, panel_bottom],
        );
        place(
            &mut root,
            "/Name Line/Name",
            [
                details_x,
                content_top,
                details_x + details_w,
                content_top + 28.1,
            ],
        );
        place(
            &mut root,
            "/Name Line/Name Field",
            [
                details_x,
                content_top + 28.1,
                details_x + details_w,
                content_top + 63.5,
            ],
        );
        place(
            &mut root,
            "/Name Line/ExistingBasicScenario",
            [
                details_x,
                content_top + 63.5,
                details_x + details_w,
                content_top + 89.6,
            ],
        );
        place(
            &mut root,
            "/Name Line/ExistingScenario",
            [
                details_x,
                content_top + 63.5,
                details_x + details_w,
                content_top + 110.7,
            ],
        );
        place(
            &mut root,
            "/Description Section/Name (1)",
            [
                details_x,
                content_top + 110.0,
                details_x + details_w,
                content_top + 138.1,
            ],
        );
        place(
            &mut root,
            "/DescriptionItem/Description",
            [
                details_x,
                content_top + 138.1,
                details_x + details_w,
                panel_bottom - 25.0,
            ],
        );
        place(
            &mut root,
            "/Preview Section/Name",
            [
                preview_x,
                content_top,
                preview_x + 400.0,
                content_top + 28.1,
            ],
        );
        place(
            &mut root,
            "/Preview Section/preview",
            [
                preview_x,
                content_top + 28.1,
                preview_x + 400.0,
                content_top + 428.1,
            ],
        );
        // The typed texts (the focused field shows a caret) and the placeholders.
        let name_text = if focus == 0 {
            format!("{}|", editor.save_name)
        } else {
            editor.save_name.clone()
        };
        let desc_text = if focus == 1 {
            format!("{}|", editor.save_description)
        } else {
            editor.save_description.clone()
        };
        for (field, text) in [("Name Field", name_text), ("Description", desc_text)] {
            if let Some(node) = find_mut(&mut root, &|n: &Node| n.name == field) {
                let empty = text.is_empty();
                if let Some(placeholder) = find_mut(node, &|n: &Node| n.name == "Placeholder") {
                    set_active(placeholder, empty);
                }
                if let Some(content) = find_mut(node, &|n: &Node| n.name == "Text") {
                    set_text(content, &text);
                }
            }
        }
        // The preview image is a texture drawn over the frame; without a screenshot only the
        // frame and the "no preview" text show.
        if let Some(node) = find_mut(&mut root, &|n: &Node| n.path.ends_with("/preview/Image")) {
            set_active(node, false);
        }
        // Warnings under the name.
        let official = menu.editor_name_problem() == Some("official");
        let exists = !official && menu.editor_name_exists();
        if let Some(node) = find_mut(&mut root, &|n: &Node| n.name == "ExistingBasicScenario") {
            set_active(node, official);
        }
        if let Some(node) = find_mut(&mut root, &|n: &Node| n.name == "ExistingScenario") {
            set_active(node, exists);
        }
        // The Save button greys out while the name is unusable.
        if !valid && let Some(node) = find_mut(&mut root, &|n: &Node| n.name == "SaveSettings") {
            for image in &mut node.images {
                if let Some(color) = image.color.as_mut() {
                    color.a = 0.5;
                }
            }
        }
        let Some(layer) = self.replay_replacement(
            "main_menu_scenario_editor",
            &is_saver,
            &root,
            ui,
            width,
            height,
            metrics,
            43_100,
        ) else {
            return;
        };
        frame.draws.extend(layer.draws);
        let hit = |frame: &mut UiFrame, id: UiId, rect: [f32; 4], action: UiAction| {
            frame.hit(
                id,
                Vec2::new(rect[0] * scale, rect[1] * scale),
                Vec2::new(rect[2] * scale, rect[3] * scale),
                action,
            );
        };
        hit(
            frame,
            43_300,
            [details_x, content_top + 28.1, details_w, 35.4],
            UiAction::Menu(crate::ui::MenuCommand::EditorFocus(0)),
        );
        hit(
            frame,
            43_301,
            [
                details_x,
                content_top + 138.1,
                details_w,
                panel_bottom - 25.0 - content_top - 138.1,
            ],
            UiAction::Menu(crate::ui::MenuCommand::EditorFocus(1)),
        );
        if valid {
            hit(
                frame,
                43_302,
                [px, py, 70.0, 70.0],
                UiAction::Menu(crate::ui::MenuCommand::EditorAnswer(true)),
            );
        }
        hit(
            frame,
            43_303,
            [px + 1030.0, py, 70.0, 70.0],
            UiAction::Menu(crate::ui::MenuCommand::EditorAnswer(false)),
        );
    }

    /// Builds the exact game title menu (or settings menu) without an in-world
    /// `App`. Used by the interactive title screen and `--menu --screenshot`.
    #[must_use]
    pub fn build_menu_frame(
        &self,
        settings: bool,
        ui: &UiAssets,
        width: f32,
        height: f32,
        metrics: Option<&crate::text_sdf::SdfFonts>,
    ) -> Option<UiFrame> {
        let session = if settings {
            self.menu_settings.as_ref()
        } else {
            self.menu_title.as_ref()
        }?;
        let session = filter_menu_session(session);
        let session_name = if settings {
            "main_menu_settings"
        } else {
            "main_menu_title"
        };
        Some(self.build_session_frame(session_name, &session, None, metrics, ui, width, height))
    }

    /// World/pixel rectangles of the five menu buttons we wire to actions.
    /// Order matches the game's main menu: Continue, New, Load, Bibite
    /// Library (`Editor` node), Settings, Achievements, Additional Tools
    /// (`Section/More`), Credits, Quit.
    #[must_use]
    pub fn menu_button_rects(&self, width: f32, height: f32) -> Vec<(Vec2, Vec2)> {
        let Some(session) = self.menu_title.as_ref() else {
            return Vec::new();
        };
        let scale = UiFrame::scaler(width, height);
        let paths = [
            "UI/WelcomeScreen/Main Holder/ButtonsPanel/Continue",
            "UI/WelcomeScreen/Main Holder/ButtonsPanel/NewSim",
            "UI/WelcomeScreen/Main Holder/ButtonsPanel/LoadSim",
            "UI/WelcomeScreen/Main Holder/ButtonsPanel/Editor",
            "UI/WelcomeScreen/Main Holder/ButtonsPanel/Settings",
            "UI/WelcomeScreen/Main Holder/ButtonsPanel/Section/Achievements",
            "UI/WelcomeScreen/Main Holder/ButtonsPanel/Section/More",
            "UI/WelcomeScreen/Main Holder/ButtonsPanel/Section/Credits",
            "UI/WelcomeScreen/Main Holder/ButtonsPanel/Quit",
        ];
        paths
            .iter()
            .filter_map(|path| compose_path(session, path))
            .map(|rect| {
                (
                    Vec2::new(rect.pos.x * scale, rect.pos.y * scale),
                    Vec2::new(rect.size.x * scale, rect.size.y * scale),
                )
            })
            .collect()
    }

    /// `true` when the frame will use the selected-bibite session (expanded
    /// Stats panel); the caller must then skip the curated inspector overlay.
    #[must_use]
    pub fn uses_inspector_session(&self, app: &App) -> bool {
        app.selection.is_some() && self.tab_session(app.inspector_tab).is_some()
    }

    /// Runtime session for one inspector tab, when captured.
    #[must_use]
    pub fn tab_session(&self, tab: InspectorTab) -> Option<&RuntimeSession> {
        match tab {
            InspectorTab::Stats => self.inspector_session.as_ref(),
            InspectorTab::Genes => self.genes_session.as_ref(),
            InspectorTab::Biology => self.biology_session.as_ref(),
            InspectorTab::Brain => self.brain_session.as_ref(),
        }
    }

    /// Every captured active control across all runtime sessions, with its
    /// composed screen rect at `width`x`height`.
    #[allow(dead_code)] // P0 registry consumed by the framework follow-up
    #[must_use]
    pub fn controls(&self, width: f32, height: f32) -> Vec<RuntimeControl> {
        self.controls_in(width, height, false)
    }

    /// [`Self::controls`] over every session of the dump: the ones parsed on first use (`lazy_sessions`) are parsed now.
    #[must_use]
    #[allow(dead_code)] // the registry audit in the tests
    pub fn controls_all(&self, width: f32, height: f32) -> Vec<RuntimeControl> {
        self.controls_in(width, height, true)
    }

    fn controls_in(&self, width: f32, height: f32, with_lazy: bool) -> Vec<RuntimeControl> {
        let mut out = Vec::new();
        for (name, session) in self.named_sessions() {
            collect_session_controls(&name, session, width, height, &mut out);
        }
        if with_lazy {
            for name in self.lazy_sessions.keys() {
                if let Some(session) = self.session_by_name(name) {
                    collect_session_controls(name, session, width, height, &mut out);
                }
            }
        }
        // Some canvas captures expose the same node through more than one root
        // object; keep one registry entry per stable id, preferring the copy
        // that carries captured value/action metadata.
        let mut by_id: std::collections::BTreeMap<u64, RuntimeControl> =
            std::collections::BTreeMap::new();
        for mut control in out.drain(..) {
            match by_id.entry(control.id) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(control);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let existing = entry.get_mut();
                    if existing.value.is_none() {
                        existing.value = control.value.take();
                    }
                    if existing.action.is_none() {
                        existing.action = control.action.take();
                    }
                    if existing.binding.is_none() {
                        existing.binding = control.binding;
                    }
                }
            }
        }
        out.extend(by_id.into_values());
        for control in &out {
            if control.action.is_none() && control.binding.is_none() {
                log_unmapped_control(control);
            }
        }
        out
    }

    /// Controls for one named session (empty when the session is absent).
    #[must_use]
    #[allow(dead_code)] // P0 registry consumed by the framework follow-up
    pub fn session_controls(&self, session: &str, width: f32, height: f32) -> Vec<RuntimeControl> {
        self.controls(width, height)
            .into_iter()
            .filter(|control| control.session == session)
            .collect()
    }

    #[allow(dead_code)]
    fn named_sessions(&self) -> Vec<(String, &RuntimeSession)> {
        let mut sessions = Vec::new();
        if !self.native {
            sessions.push(("default_world".to_owned(), &self.session));
        }
        if let Some(session) = &self.inspector_session {
            sessions.push(("default_world_inspector".to_owned(), session));
        }
        if let Some(session) = &self.genes_session {
            sessions.push(("default_world_genes".to_owned(), session));
        }
        if let Some(session) = &self.biology_session {
            sessions.push(("default_world_biology".to_owned(), session));
        }
        if let Some(session) = &self.brain_session {
            sessions.push(("default_world_brain".to_owned(), session));
        }
        if let Some(session) = &self.menu_title {
            sessions.push(("main_menu_title".to_owned(), session));
        }
        if let Some(session) = &self.menu_settings {
            sessions.push(("main_menu_settings".to_owned(), session));
        }
        for (name, session) in &self.extra_sessions {
            sessions.push((name.clone(), session));
        }
        sessions
    }

    /// Named session lookup (fixed + functional/extra captures).
    #[must_use]
    pub fn session_by_name(&self, name: &str) -> Option<&RuntimeSession> {
        if let Some((_, session)) = self
            .named_sessions()
            .into_iter()
            .find(|(candidate, _)| candidate == name)
        {
            return Some(session);
        }
        let (file, cell) = self.lazy_sessions.get(name)?;
        cell.get_or_init(|| match load_session_file(file) {
            Ok(session) => Some(session),
            Err(error) => {
                eprintln!("ui: session {name}: {error}");
                None
            }
        })
        .as_ref()
    }

    /// All session names present in the dump.
    #[must_use]
    pub fn session_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .named_sessions()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        names.extend(self.lazy_sessions.keys().cloned());
        names
    }

    /// Hotkey reference blocks captured on the Settings/Hotkeys page. The
    /// current atlas stores the full hotkey list as text on one scroll view,
    /// so this exposes its path and lines without inventing per-key controls.
    #[allow(dead_code)] // settings panel follow-up
    #[must_use]
    pub fn hotkey_blocks(&self, session: &str) -> Vec<HotkeyBlock> {
        let Some(session) = self.session_by_name(session) else {
            return Vec::new();
        };
        let mut blocks = Vec::new();
        for root in active_canvas_roots(session) {
            collect_hotkey_blocks(root, &mut blocks);
        }
        blocks
    }

    /// Builds a frame for any named session (functional menu/tool panels).
    #[must_use]
    pub fn build_named_session_frame(
        &self,
        name: &str,
        ui: &UiAssets,
        width: f32,
        height: f32,
    ) -> Option<UiFrame> {
        let session = self.session_by_name(name)?;
        Some(self.build_session_frame(name, session, None, None, ui, width, height))
    }

    /// Builds a menu-like session without gameplay/loading canvases.
    #[must_use]
    pub fn build_menu_like_session_frame(
        &self,
        name: &str,
        ui: &UiAssets,
        width: f32,
        height: f32,
        metrics: Option<&crate::text_sdf::SdfFonts>,
    ) -> Option<UiFrame> {
        let session = self.session_by_name(name)?;
        let filtered = filter_menu_session(session);
        Some(self.build_session_frame(name, &filtered, None, metrics, ui, width, height))
    }

    /// Builds a named session using game SDF advances for text layout.
    #[must_use]
    pub fn build_named_session_frame_with_metrics(
        &self,
        name: &str,
        ui: &UiAssets,
        width: f32,
        height: f32,
        metrics: &crate::text_sdf::SdfFonts,
    ) -> Option<UiFrame> {
        let session = self.session_by_name(name)?;
        Some(self.build_session_frame(name, session, None, Some(metrics), ui, width, height))
    }

    fn sessions(&self) -> impl Iterator<Item = &RuntimeSession> {
        [
            Some(&self.session),
            self.inspector_session.as_ref(),
            self.genes_session.as_ref(),
            self.biology_session.as_ref(),
            self.brain_session.as_ref(),
            self.menu_title.as_ref(),
            self.menu_settings.as_ref(),
        ]
        .into_iter()
        .flatten()
    }

    fn frame_session(&self, app: &App) -> &RuntimeSession {
        if app.inspector_visible()
            && let Some(session) = self.tab_session(app.inspector_tab)
        {
            session
        } else {
            &self.session
        }
    }

    #[allow(clippy::too_many_arguments)] // layout inputs stay explicit
    fn build_session_frame(
        &self,
        session_name: &str,
        session: &RuntimeSession,
        app: Option<&App>,
        metrics: Option<&crate::text_sdf::SdfFonts>,
        ui: &UiAssets,
        width: f32,
        height: f32,
    ) -> UiFrame {
        self.build_session_frame_filtered(
            session_name,
            session,
            app,
            metrics,
            ui,
            width,
            height,
            None,
            100,
            None,
        )
    }

    /// Paths that are active in `name` but not in the base world session (and
    /// not part of the inspector panels): the panel a toolbar button opens.
    fn overlay_paths(&self, name: &str, root: &str) -> Option<HashSet<String>> {
        self.overlay_paths_under(name, &[root])
    }

    /// [`Self::overlay_paths`] for several roots at once.
    fn overlay_paths_under(&self, name: &str, roots: &[&str]) -> Option<HashSet<String>> {
        let session = self.session_by_name(name)?;
        let mut base = Vec::new();
        for canvas in &self.session.canvases {
            collect_active_paths(canvas, &mut base);
        }
        let base: HashSet<String> = base.into_iter().collect();
        let mut extra = Vec::new();
        for canvas in &session.canvases {
            collect_active_paths(canvas, &mut extra);
        }
        Some(
            extra
                .into_iter()
                .filter(|path| !base.contains(path) && roots.iter().any(|root| path.contains(root)))
                .collect(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_session_frame_filtered(
        &self,
        session_name: &str,
        session: &RuntimeSession,
        app: Option<&App>,
        metrics: Option<&crate::text_sdf::SdfFonts>,
        ui: &UiAssets,
        width: f32,
        height: f32,
        only: Option<&HashSet<String>>,
        first_hit: UiId,
        menu: Option<&crate::menu::MenuState>,
    ) -> UiFrame {
        let scale = UiFrame::scaler(width, height);
        // The canvas is `CanvasScaler.scaleFactor` = totalScale pixels per unit, so it is the window over that (1920 x 1080 only at the capture's
        // window): the corner-anchored panels follow its edges.
        let reference = Vec2::new(width / scale, height / scale);
        let mut frame = UiFrame::new(scale);
        // The Stats panel follows the state of the target (parents, traits, egg, dead): see `stats_states`.
        let stats = match (session_name, app) {
            ("default_world_inspector", Some(app)) => crate::stats_states::StatsState::of(app)
                .and_then(|(state, texts)| {
                    fn find(node: &Node) -> Option<&Node> {
                        if node.path.ends_with("/BibitePanels/StatsPanel") {
                            return Some(node);
                        }
                        node.children.iter().find_map(find)
                    }
                    let panel = session.canvases.iter().find_map(find)?;
                    let fonts = metrics.or_else(|| crate::stats_states::fallback_fonts(&ui.root));
                    Some(crate::stats_states::StatsLayout::new(
                        panel, &state, &texts, fonts,
                    ))
                }),
            _ => None,
        };
        let mut context = FrameContext {
            ui,
            app,
            metrics,
            session: session_name.to_owned(),
            frame: &mut frame,
            scale,
            reference,
            emitted: 0,
            next_hit: first_hit,
            last_gene: None,
            only,
            toggle_on: None,
            toggle_meta: None,
            menu,
            setting_row: None,
            local_scale: 1.0,
            details_shift: 0.0,
            scroll_clip: None,
            shift: [None; 2],
            depth: 0,
            stats,
            brain: match (session_name, app) {
                ("default_world_brain", Some(app)) => {
                    fn find(node: &Node) -> Option<&Node> {
                        if node.path.ends_with("/BibitePanels/BrainPanel") {
                            return Some(node);
                        }
                        node.children.iter().find_map(find)
                    }
                    app.inspector_brain
                        .as_ref()
                        .filter(|state| {
                            Some(state.target) == app.selection.as_ref().map(|panel| panel.target)
                        })
                        .zip(session.canvases.iter().find_map(find))
                        .map(|(state, capture)| {
                            let fonts =
                                metrics.or_else(|| crate::stats_states::fallback_fonts(&ui.root));
                            crate::inspector_brain::panel_layout(
                                capture,
                                state,
                                &|_, node| {
                                    node.texts
                                        .first()
                                        .map(|text| live_text(app, node, text, &mut None))
                                },
                                fonts,
                            )
                        })
                }
                _ => None,
            },
        };
        for root in active_canvas_roots(session) {
            if session_name.contains("menu")
                && (root.path.eq_ignore_ascii_case("BackGround")
                    || (menu.is_none() && root.path.contains("LoadingPanel")))
            {
                continue;
            }
            if !is_canvas(root) {
                // Plain RectTransform wrappers (world-space helpers) are not
                // screen UI; skip them.
                continue;
            }
            let composed = compose_root(root, context.reference, self.semantics);
            if composed.size.x > context.reference.x * 4.0
                || composed.size.y > context.reference.y * 4.0
            {
                // World-space/full-world wrappers (e.g. WeightDisplayHolder).
                continue;
            }
            emit_node(&mut context, root, composed, self.semantics);
        }
        if app.is_some() && only.is_none() {
            emit_ruler(&mut context, self, session);
        }
        context.frame.draws.shrink_to_fit();
        context.frame.hits.shrink_to_fit();
        context.frame.actions.shrink_to_fit();
        frame
    }
}

/// Title, text and button labels (dismiss first, accept second) of a confirmation, exactly as
/// the game's `DisplayChoiceDialog` / `DisplayActionWarning` calls give them.
fn confirm_texts(
    kind: &crate::app::ConfirmKind,
    zone_name: &str,
) -> (&'static str, String, [&'static str; 2]) {
    use crate::app::ConfirmKind;
    match kind {
        // `ZoneSettingsHandle.DeleteZone` and `ZonesEditorPanel.DeleteAllZones`.
        ConfirmKind::DeleteZone(_) => (
            "Delete Zone",
            format!("Are you sure you want to delete {zone_name}?\nYou will lose all data"),
            ["Cancel", "YES"],
        ),
        ConfirmKind::ClearZones => (
            "Delete All Zones",
            "Are you sure you want to delete all zones? You will lose all data".to_owned(),
            ["Cancel", "YES"],
        ),
        // `SettingsChangerHandle.DeleteChanger` and `SettingsChangersEditor.DeleteAllChangers`.
        ConfirmKind::DeleteAnimator(_) => (
            "Delete Settings Animator",
            format!("Are you sure you want to delete {zone_name}?\nYou will lose all data"),
            ["Cancel", "YES"],
        ),
        ConfirmKind::ClearAnimators => (
            "Delete All Settings Animators",
            "Are you sure you want to delete all settings animators? You will lose all data".to_owned(),
            ["Cancel", "YES"],
        ),
        ConfirmKind::KillHalf => (
            "Killing half the population",
            "You're about to kill half of all bibites (each bibites will have a 50% chance of being killed, results may vary).\nAre you sure???".to_owned(),
            ["Cancel", "YES"],
        ),
        ConfirmKind::ClearDefaultBibites => (
            "Delete All Default Bibites",
            "Are you sure you want to delete all default bibites? You will lose all data.".to_owned(),
            ["Cancel", "YES"],
        ),
        ConfirmKind::RemoveHalfPellets => (
            "Removing half of all pellets",
            "You're about to remove half of all pellets (each pellet will have a 50% chance of being removed, results may vary). Depending on your numbers of pellets the screen might freeze a little while doing so.\nAre you sure???".to_owned(),
            ["Cancel", "YES"],
        ),
        ConfirmKind::OverwriteSave(_) => (
            "Are you SURE???",
            "You are about to delete a previous save file, are you sure you want to do this?".to_owned(),
            ["Cancel", "OK"],
        ),
        ConfirmKind::DeleteSave(_) => (
            "Deleting Save File",
            "Are you sure you want to delete this save file?".to_owned(),
            ["Cancel", "Yes"],
        ),
        // `SpeciesInfoPanel.PruneSpecies`.
        ConfirmKind::PruneSpecies(_) => (
            crate::warnings::ActionWarning::PruneSpecies.shown_title(),
            crate::warnings::ActionWarning::PruneSpecies.text().to_owned(),
            ["Cancel", crate::warnings::ActionWarning::PruneSpecies.proceed_label()],
        ),
        ConfirmKind::RestartChallenge => (
            "Retry Challenge",
            "You will lose all progress on this challenge.\n\nAre you sure?".to_owned(),
            ["Cancel", "YES"],
        ),
        // No dialog in the game: the kill buttons act at once (kept for the fallback HUD).
        ConfirmKind::KillSelected => (
            "Kill",
            "Kill this bibite? Are you sure???".to_owned(),
            ["Cancel", "YES"],
        ),
    }
}

/// The bottom-left toolbar: (capture session of the open panel, panel root for the replay,
/// button path, panel path). The panels are `OnHoverShow` targets of their buttons.
const TOOLBAR: [(&str, &str, &str, &str); 3] = [
    (
        "main_world_tool_placer",
        "/BottomLeft/PlacerButton",
        "UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/PlacerButton",
        "UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/PlacerButton/Placers",
    ),
    (
        "main_world_tool_user_actions",
        "/BottomLeft/UserActions",
        "UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/UserActions",
        "UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/UserActions/container",
    ),
    (
        "main_world_tool_view",
        "/BottomLeft/ViewButton",
        "UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/ViewButton",
        "UICanvas/MainUI/Layer0/UI Buttons/BottomLeft/ViewButton/VisibilityPanel",
    ),
];

/// Top-level active canvases (excluding canvases nested under another one).
/// Converts a native recapture node (`components` array) into the atlas node
/// shape used by the frame composer.
fn convert_native_node(value: &serde_json::Value, parent_active: bool) -> Node {
    let active_self = value
        .get("active_self")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let active = parent_active && active_self;
    let mut node = Node {
        name: value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        path: value
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        // `activeSelf`: a node below an inactive one keeps its own flag (the Stats panel's egg section is captured inactive
        // and the layout solver needs to know which of its nodes the prefab itself has active).
        active: active_self,
        active_in_hierarchy: active,
        sibling_index: value
            .get("sibling_index")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0) as i32,
        rect: value
            .get("rect")
            .cloned()
            .and_then(|rect| serde_json::from_value(rect).ok())
            .unwrap_or_default(),
        native_rect: value.get("screen_bbox").and_then(|bbox| {
            let get = |key: &str| {
                bbox.get(key)
                    .and_then(serde_json::Value::as_f64)
                    .map(|v| v as f32)
            };
            Some([get("x0")?, get("y0")?, get("x1")?, get("y1")?])
        }),
        images: Vec::new(),
        texts: Vec::new(),
        canvas_scalers: Vec::new(),
        layouts: Vec::new(),
        controls: Vec::new(),
        children: Vec::new(),
    };
    if let Some(components) = value
        .get("components")
        .and_then(serde_json::Value::as_array)
    {
        for component in components {
            convert_native_component(component, &mut node);
        }
    }
    if let Some(children) = value.get("children").and_then(serde_json::Value::as_array) {
        node.children = children
            .iter()
            .map(|child| convert_native_node(child, active))
            .collect();
    }
    node
}

/// Converts one captured component into the node's typed component lists.
fn convert_native_component(component: &serde_json::Value, node: &mut Node) {
    use serde_json::json;
    let kind = component
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if kind == "UnityEngine.Canvas" {
        if let Ok(control) = serde_json::from_value::<ControlComponent>(component.clone()) {
            node.controls.push(control);
        }
    } else if kind == "UnityEngine.UI.CanvasScaler" {
        if let Ok(scaler) = serde_json::from_value::<CanvasScaler>(component.clone()) {
            node.canvas_scalers.push(scaler);
        }
    } else if kind.contains("Button")
        || kind.contains("Selectable")
        || kind.contains("Toggle")
        || kind.contains("Slider")
        || kind.contains("Scrollbar")
        || kind.contains("ScrollRect")
        || kind.contains("Dropdown")
        || kind.contains("InputField")
    {
        if let Ok(control) = serde_json::from_value::<ControlComponent>(component.clone()) {
            node.controls.push(control);
        }
    } else if kind.contains("Image") {
        let sprite = component
            .get("sprite")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let image = json!({
            "type": kind,
            "sprite": if sprite.is_empty() { serde_json::Value::Null } else { json!({ "name": sprite }) },
            "color": component.get("color").cloned().unwrap_or(serde_json::Value::Null),
            "image_type": component.get("image_type").cloned().unwrap_or(serde_json::Value::Null),
            "fill_method": component.get("fill_method").cloned().unwrap_or(serde_json::Value::Null),
            "fill_amount": component.get("fill_amount").cloned().unwrap_or(serde_json::Value::Null),
            "runtime_sprite_border": component.get("sprite_border").cloned().unwrap_or(serde_json::Value::Null),
            "sprite_ppu": component.get("sprite_pixels_per_unit").cloned().unwrap_or(serde_json::Value::Null),
            "disabled": component.get("enabled").and_then(serde_json::Value::as_bool) == Some(false),
        });
        if let Ok(image) = serde_json::from_value::<ImageComponent>(image) {
            node.images.push(image);
        }
    } else if kind.contains("TextMeshPro") {
        let font = component
            .get("font")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let text = json!({
            "type": kind,
            "text": component.get("text").cloned().unwrap_or(serde_json::Value::Null),
            "font": if font.is_empty() { serde_json::Value::Null } else { json!({ "name": font }) },
            "font_size": component.get("font_size").cloned().unwrap_or(serde_json::Value::Null),
            "alignment": component.get("alignment").cloned().unwrap_or(serde_json::Value::Null),
            "color": component.get("color").cloned().unwrap_or(serde_json::Value::Null),
            "word_wrap": component.get("word_wrap").cloned().unwrap_or(serde_json::Value::Null),
            "font_style": component.get("font_style").cloned().unwrap_or(serde_json::Value::Null),
            "text_style": component.get("text_style").cloned().unwrap_or(serde_json::Value::Null),
            "rich_text": component.get("rich_text").cloned().unwrap_or(serde_json::Value::Null),
            "character_spacing": component.get("character_spacing").cloned().unwrap_or(serde_json::Value::Null),
            "line_spacing": component.get("line_spacing").cloned().unwrap_or(serde_json::Value::Null),
            "margin": component.get("margin").cloned().unwrap_or(serde_json::Value::Null),
        });
        if let Ok(text) = serde_json::from_value::<TextComponent>(text) {
            node.texts.push(text);
        }
    }
}

/// A captured node deserialised straight into typed lists: each component is converted as
/// it is read, so no `serde_json::Value` tree of the whole (tens of MB) session is built.
#[derive(Default, Deserialize)]
struct StreamedNode {
    #[serde(default, deserialize_with = "null_string")]
    name: String,
    #[serde(default, deserialize_with = "null_string")]
    path: String,
    #[serde(default = "default_true")]
    active_self: bool,
    #[serde(default)]
    sibling_index: Option<i64>,
    #[serde(default, deserialize_with = "null_rect")]
    rect: RectTransform,
    #[serde(default)]
    screen_bbox: Option<StreamedBox>,
    #[serde(default, deserialize_with = "streamed_components")]
    components: StreamedComponents,
    #[serde(default, deserialize_with = "null_vec")]
    children: Vec<StreamedNode>,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct StreamedBox {
    x0: Option<f64>,
    y0: Option<f64>,
    x1: Option<f64>,
    y1: Option<f64>,
}

/// The typed component lists of one node.
#[derive(Default)]
struct StreamedComponents {
    holder: Option<Box<Node>>,
}

fn streamed_components<'de, D>(deserializer: D) -> Result<StreamedComponents, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = StreamedComponents;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a component list")
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(StreamedComponents::default())
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(StreamedComponents::default())
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut holder = Node::default();
            while let Some(component) = seq.next_element::<serde_json::Value>()? {
                convert_native_component(&component, &mut holder);
            }
            Ok(StreamedComponents {
                holder: Some(Box::new(holder)),
            })
        }
    }
    deserializer.deserialize_any(Visitor)
}

impl StreamedNode {
    /// `active_in_hierarchy = parent_active && active_self`, as `convert_native_node` does; `active` is the node's own flag.
    fn finish(self, parent_active: bool) -> Node {
        let active = parent_active && self.active_self;
        let mut node = self
            .components
            .holder
            .map_or_else(Node::default, |holder| *holder);
        node.name = self.name;
        node.path = self.path;
        node.active = self.active_self;
        node.active_in_hierarchy = active;
        node.sibling_index = self.sibling_index.unwrap_or(0) as i32;
        node.rect = self.rect;
        node.native_rect = self.screen_bbox.and_then(|bbox| {
            Some([
                bbox.x0? as f32,
                bbox.y0? as f32,
                bbox.x1? as f32,
                bbox.y1? as f32,
            ])
        });
        node.children = self
            .children
            .into_iter()
            .map(|child| child.finish(active))
            .collect();
        node
    }
}

/// Removes the gameplay/backdrop and loading-screen canvases from a menu session.
fn filter_menu_session(session: &RuntimeSession) -> RuntimeSession {
    fn prune(node: &mut Node) {
        node.children.retain(|child| {
            let path = child.path.as_str();
            !(path == "Title Screen/Title" || path == "Title Screen/Title (1)")
        });
        for child in &mut node.children {
            prune(child);
        }
    }
    let mut filtered = session.clone();
    filtered.canvases.retain(|canvas| {
        let path = canvas.path.as_str();
        !(path.eq_ignore_ascii_case("BackGround")
            || path.eq_ignore_ascii_case("LoadingPanel")
            || path.to_ascii_lowercase().contains("loadingpanel"))
    });
    for canvas in &mut filtered.canvases {
        prune(canvas);
    }
    filtered
}

fn active_canvas_roots(session: &RuntimeSession) -> Vec<&Node> {
    let active: Vec<&Node> = session
        .canvases
        .iter()
        .filter(|node| node.active_in_hierarchy && is_canvas(node))
        .collect();
    // Skip a canvas root that is physically contained in another active canvas
    // root (avoid duplicate drawing); keep separate root objects even when their
    // paths share a prefix (functional panels are captured as separate canvases).
    let descendant_paths: Vec<std::collections::HashSet<&str>> = active
        .iter()
        .map(|node| {
            let mut paths = std::collections::HashSet::new();
            collect_active_path_set(node, &mut paths);
            paths
        })
        .collect();
    active
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            !descendant_paths
                .iter()
                .enumerate()
                .any(|(other, paths)| other != *index && paths.contains(node.path.as_str()))
        })
        .map(|(_, node)| *node)
        .collect()
}

#[allow(dead_code)] // used by hotkey_blocks
fn collect_hotkey_blocks(node: &Node, out: &mut Vec<HotkeyBlock>) {
    if !node.active_in_hierarchy {
        return;
    }
    if node.path.contains("/SettingsPanel/PagesPanels/Hotkeys/") && !node.texts.is_empty() {
        let lines: Vec<String> = node
            .texts
            .iter()
            .flat_map(|text| text.text.lines())
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        if !lines.is_empty() {
            out.push(HotkeyBlock {
                path: node.path.clone(),
                lines,
            });
        }
        return;
    }
    for child in &node.children {
        collect_hotkey_blocks(child, out);
    }
}

fn collect_active_path_set<'a>(node: &'a Node, paths: &mut std::collections::HashSet<&'a str>) {
    if !node.active_in_hierarchy {
        return;
    }
    paths.insert(node.path.as_str());
    for child in &node.children {
        collect_active_path_set(child, paths);
    }
}

struct FrameContext<'a> {
    ui: &'a UiAssets,
    app: Option<&'a App>,
    /// Game SDF advances for center/right/line layout when available.
    metrics: Option<&'a crate::text_sdf::SdfFonts>,
    session: String,
    frame: &'a mut UiFrame,
    scale: f32,
    reference: Vec2,
    emitted: usize,
    next_hit: UiId,
    /// Gene index of the most recent `GeneLine/geneName` text (the following
    /// `value` / `difference` nodes belong to it).
    last_gene: Option<usize>,
    /// Overlay mode: only nodes whose path is listed are drawn (the rest of the
    /// tree is still walked for layout).
    only: Option<&'a HashSet<String>>,
    /// Live state of the settings toggle row being emitted (hides its checkmark).
    toggle_on: Option<bool>,
    /// Index of the scenario setting behind the toggle row being emitted.
    toggle_meta: Option<usize>,
    /// Live title-menu state (native menu panels only).
    menu: Option<&'a crate::menu::MenuState>,
    /// Settings row (index into `user_settings::DEFS`) being emitted.
    setting_row: Option<usize>,
    /// Accumulated `localScale` of the popup holder (its TMP text scales with it: the
    /// `Popuholder` is scaled 1.35 in the game, the captured font sizes are unscaled).
    local_scale: f32,
    /// The Stats panel in the state of the selected bibite or egg (`default_world_inspector` only).
    stats: Option<crate::stats_states::StatsLayout>,
    /// The Brain panel laid out for the target's diagram and species (`default_world_brain` only).
    brain: Option<crate::panel_layout::PanelLayout>,
    /// How far the scenario's description (wrapped for the text it shows now) pushes the stars and the high score under it, in
    /// reference px (the New Game screen's `Details` is a vertical layout group; the capture has the lines of its own text).
    details_shift: f32,
    /// The rows (reference px, y down: top and bottom) of the view of the scrolling panel whose text is being emitted: its lines are drawn when they lie
    /// inside (a text cannot be cut by a mask, so a line half out of the view is left out).
    scroll_clip: Option<[f32; 2]>,
    /// How far the canvas is bigger than the capture's 1920 x 1080 and where the node being emitted sits in that: the first anchored (non-stretched) ancestor
    /// of each axis decides (`anchor * extra` across, `(1 - anchor) * extra` down), because the captured boxes were laid out for 1920 x 1080 only.
    shift: [Option<f32>; 2],
    /// How deep in its canvas root the node being emitted is (0 for the root).
    depth: u32,
}

fn count_nodes(node: &Node, stats: &mut RuntimeStats) {
    stats.nodes += 1;
    if node.active_in_hierarchy {
        stats.active_nodes += 1;
        stats.active_images += node.images.len();
        stats.active_texts += node.texts.len();
    }
    for child in &node.children {
        count_nodes(child, stats);
    }
}

fn collect_sprite_files(node: &Node, files: &mut Vec<String>) {
    if node.active_in_hierarchy {
        for image in &node.images {
            let Some(sprite) = &image.sprite else {
                continue;
            };
            if let Some(manifest) = &sprite.manifest {
                if !manifest.file.is_empty() {
                    files.push(manifest.file.clone());
                    continue;
                }
            }
            if let Some(file) = &sprite.file {
                files.push(file.clone());
            }
        }
        for child in &node.children {
            collect_sprite_files(child, files);
        }
    }
}

fn collect_active_paths(node: &Node, paths: &mut Vec<String>) {
    if node.active_in_hierarchy {
        paths.push(node.path.clone());
    }
    for child in &node.children {
        collect_active_paths(child, paths);
    }
}

/// Derives a stable panel key/prefix from one core-panel path.
fn panel_key(path: &str) -> (String, String) {
    for marker in ["Layer1/Panels/", "Layer0/"] {
        if let Some(index) = path.find(marker) {
            let rest = &path[index + marker.len()..];
            let segments: Vec<&str> = rest.split('/').collect();
            let name = match marker {
                "Layer1/Panels/" => {
                    if segments.first() == Some(&"Right Panels") {
                        format!(
                            "Right Panels/{}",
                            segments.get(1).copied().unwrap_or("Panels")
                        )
                    } else if segments.first() == Some(&"BibitePanels") {
                        format!(
                            "BibitePanels/{}",
                            segments.get(1).copied().unwrap_or("Panels")
                        )
                    } else {
                        segments.first().copied().unwrap_or("Panels").to_owned()
                    }
                }
                _ => {
                    if segments.first() == Some(&"UI Buttons") {
                        format!(
                            "HUD/UI Buttons/{}",
                            segments.get(1).copied().unwrap_or("Buttons")
                        )
                    } else if segments.first() == Some(&"Controls") {
                        format!(
                            "HUD/Controls/{}",
                            segments.get(1).copied().unwrap_or("TopCenter")
                        )
                    } else {
                        format!("HUD/{}", segments.first().copied().unwrap_or("Other"))
                    }
                }
            };
            let prefix_len = path.len() - rest.len() + panel_rest_len(rest, &name);
            let prefix = path[..prefix_len.min(path.len())].to_owned();
            return (name, prefix);
        }
    }
    ("Other".to_owned(), path.to_owned())
}

fn panel_rest_len(rest: &str, name: &str) -> usize {
    let depth = if name.starts_with("Right Panels/")
        || name.starts_with("BibitePanels/")
        || name.starts_with("HUD/UI Buttons/")
        || name.starts_with("HUD/Controls/")
    {
        2
    } else if name.starts_with("HUD/") {
        1
    } else if rest.starts_with("Right Panels/") {
        2
    } else {
        1
    };
    rest.split('/').take(depth).map(|part| part.len() + 1).sum()
}

fn is_canvas(node: &Node) -> bool {
    node.controls
        .iter()
        .any(|control| control.r#type == "UnityEngine.Canvas")
}

/// The canvas a session is laid out in: the window's (over the UI scale) once the UI has been scaled for one, else the capture's own screen.
fn canvas_reference(session: &RuntimeSession) -> Vec2 {
    crate::ui::canvas_size().unwrap_or_else(|| Vec2::new(session.screen.width, session.screen.height))
}

/// Composes a top-level canvas root (camera-centred ScreenSpaceCamera canvas).
#[must_use]
pub fn compose_root(node: &Node, reference: Vec2, semantics: RectSemantics) -> ComposedRect {
    let rect = node.rect;
    // Native ScreenSpaceCamera roots often store sizeDelta 0 with stretched
    // anchors (0..1); the effective root size is then the full reference screen.
    let mut size = Vec2::new(
        rect.size_delta.x + (rect.anchor_max.x - rect.anchor_min.x) * reference.x,
        rect.size_delta.y + (rect.anchor_max.y - rect.anchor_min.y) * reference.y,
    );
    // A canvas the capture stored as 1920 x 1080 is the whole screen: in another window (or at another UI scale) it is the window over the scale.
    if (size.x - REFERENCE_WIDTH).abs() < 0.5 && (size.y - REFERENCE_HEIGHT).abs() < 0.5 {
        size = reference;
    }
    // Root RectTransforms are positioned relative to the camera centre
    // (ScreenSpaceCamera canvases); `local_position` is the camera-space offset.
    // Full-screen ScreenSpaceOverlay menu roots, however, store their anchor
    // position (reference centre) in `local_position`; treat those as centred.
    let mut local_x = rect.local_position.x;
    let mut local_y = rect.local_position.y;
    // The stored centre is the capture's own (960, 540), whatever the canvas is laid out for now: a root compared with the current canvas's
    // centre only was "centred" at a canvas of 1920 x 1080 and shifted by (960, 540) in any other (a UI scale other than 1, a window that is
    // not 16:9): the whole title menu was drawn, and clicked, off the screen.
    let centred_root = size.x >= reference.x * 0.9
        && size.y >= reference.y * 0.9
        && [reference, Vec2::new(REFERENCE_WIDTH, REFERENCE_HEIGHT)].into_iter().any(|centre_of| {
            (local_x - centre_of.x * 0.5).abs() < 1.0 && (local_y - centre_of.y * 0.5).abs() < 1.0
        });
    if centred_root {
        local_x = 0.0;
        local_y = 0.0;
    }
    let bottom_left = Vec2::new(
        reference.x * 0.5 + local_x - rect.pivot.x * size.x,
        reference.y * 0.5 + local_y - rect.pivot.y * size.y,
    );
    let pos = match semantics {
        RectSemantics::Local | RectSemantics::Canvas => {
            Vec2::new(bottom_left.x, reference.y - (bottom_left.y + size.y))
        }
    };
    ComposedRect {
        pos,
        size,
        scale: Vec2::new(rect.local_scale.x, rect.local_scale.y),
    }
}

/// Composes a child rect inside a parent (`top-down` coordinates).
///
/// `Local`: `anchor_min/max`-based anchored positions (Unity RectTransform
/// math). `Canvas`: `local_position` offsets in canvas space (parity
/// experiment), composed with the same pivot offset.
#[must_use]
pub fn compose_child(
    rect: &RectTransform,
    parent_size: Vec2,
    parent_pivot: Vec2,
    semantics: RectSemantics,
) -> (Vec2, Vec2) {
    let size = Vec2::new(
        (rect.anchor_max.x - rect.anchor_min.x) * parent_size.x + rect.size_delta.x,
        (rect.anchor_max.y - rect.anchor_min.y) * parent_size.y + rect.size_delta.y,
    );
    let local_min = match semantics {
        RectSemantics::Local => {
            // Unity anchor reference point: the pivot's position inside the
            // anchor rectangle (`anchor_min + (anchor_max - anchor_min)*pivot`),
            // NOT the anchor rectangle's centre. For stretched anchors this is
            // what keeps a pivot-(0,1) panel filling its parent instead of
            // drifting off-screen by half the parent size.
            let anchor_ref = Vec2::new(
                (rect.anchor_min.x + (rect.anchor_max.x - rect.anchor_min.x) * rect.pivot.x)
                    * parent_size.x,
                (rect.anchor_min.y + (rect.anchor_max.y - rect.anchor_min.y) * rect.pivot.y)
                    * parent_size.y,
            );
            Vec2::new(
                -parent_pivot.x * parent_size.x + anchor_ref.x + rect.anchored_position.x
                    - rect.pivot.x * size.x,
                -parent_pivot.y * parent_size.y + anchor_ref.y + rect.anchored_position.y
                    - rect.pivot.y * size.y,
            )
        }
        RectSemantics::Canvas => Vec2::new(
            -parent_pivot.x * parent_size.x
                + parent_pivot.x * parent_size.x
                + rect.local_position.x
                - rect.pivot.x * size.x,
            -parent_pivot.y * parent_size.y
                + parent_pivot.y * parent_size.y
                + rect.local_position.y
                - rect.pivot.y * size.y,
        ),
    };
    let topdown = Vec2::new(
        local_min.x + parent_pivot.x * parent_size.x,
        parent_size.y - (local_min.y + parent_pivot.y * parent_size.y) - size.y,
    );
    (size, topdown)
}

/// How many reference px to the right of its place in the 1920 x 1080 capture the in-world settings panel is on a canvas of `reference` px (it is
/// flush with the right edge: a wider canvas moves it right, a narrower one - a UI scale above 1 - left).
#[must_use]
pub(crate) fn dyn_panel_offset(reference: Vec2) -> f32 {
    reference.x - REFERENCE_WIDTH
}

fn emit_node(
    context: &mut FrameContext<'_>,
    node: &Node,
    composed: ComposedRect,
    semantics: RectSemantics,
) {
    let saved = context.local_scale;
    if node.path.ends_with("/Popuholder") || node.path.contains("/Popuholder/") {
        context.local_scale *= node.rect.local_scale.y;
    }
    if node.path.ends_with("/Layer0/statusbars")
        && let Some(placement) = status_bar_placement(context, node)
    {
        // The bars' texts are scaled with them (relative to the captured scale the text sizes are for).
        context.local_scale *= placement.k;
    }
    let (saved_shift, saved_depth) = (context.shift, context.depth);
    if context.depth > 0 {
        let rect = &node.rect;
        let extra = context.reference - Vec2::new(REFERENCE_WIDTH, REFERENCE_HEIGHT);
        if context.shift[0].is_none() && rect.anchor_min.x == rect.anchor_max.x {
            context.shift[0] = Some(rect.anchor_min.x * extra.x);
        }
        if context.shift[1].is_none() && rect.anchor_min.y == rect.anchor_max.y {
            context.shift[1] = Some((1.0 - rect.anchor_min.y) * extra.y);
        }
    }
    // The in-world settings panel (`Right Panels/DynamicSettingsPanel`) is flush with the right edge of the canvas and starts at the same height
    // whatever its shape: all of it (the captured chrome, the generated rows, the animator creator) moves right by what the canvas has beyond
    // 1920 reference px and nothing moves down. Its click areas (`dyn_panel::DynHit`, laid out for 1920 x 1080) get the same offset in
    // `build_dyn_panel_frame` (`dyn_panel_offset`); a node that took a vertical shift of its own (the creator's synthetic nodes did) was drawn
    // below the place it answers clicks.
    if node.path.ends_with("/Right Panels/DynamicSettingsPanel") {
        context.shift = [Some(dyn_panel_offset(context.reference)), Some(0.0)];
    }
    context.depth += 1;
    emit_node_inner(context, node, composed, semantics);
    context.local_scale = saved;
    context.shift = saved_shift;
    context.depth = saved_depth;
}

/// Whether a node is drawn: the captured state, except where the Stats panel's state decides (see `stats_states`).
fn node_shows(context: &FrameContext<'_>, node: &Node) -> bool {
    if let Some(stats) = &context.stats
        && let Some(rel) = crate::stats_states::StatsLayout::rel_of(&node.path)
        && let Some(shows) = stats.shows(rel)
    {
        return shows;
    }
    // The Biology panel's Flow bar shows the side the flow is on (`RateSliderHandle.OnSignChange`).
    if let Some(part) = crate::biology_view::flow_part(&node.path)
        && let Some(panel) = context.app.and_then(|app| app.selection.as_ref())
        && panel.internals.is_some()
    {
        use crate::biology_view::FlowPart;
        return match part {
            FlowPart::PositiveFill | FlowPart::PositiveText => panel.energy_flow > 0.0,
            FlowPart::NegativeFill | FlowPart::NegativeText => panel.energy_flow < 0.0,
        };
    }
    // A dead bibite (`BiologyPanel.CheckAndSetAliveOrDead`): "Integrity:" for "Health:", no Energy row, and the "This bibite is DEAD" line.
    if let Some(panel) = context.app.and_then(|app| app.selection.as_ref())
        && panel.internals.is_some()
        && panel.dead
    {
        if node.path.ends_with("/HealthSliderHolder/Title (Dead)") {
            return true;
        }
        if node.path.ends_with("/HealthSliderHolder/Health Title") || node.path.contains("/BodyInfoHolder/EnergySliderHolder") {
            return false;
        }
        if node.path.ends_with("/BiologyPanel/MainSection/Dead Line") || node.path.contains("/BiologyPanel/MainSection/Dead Line/") {
            return true;
        }
    }
    // The check mark of the Brain tab's "Highlight differences" toggle follows the setting.
    if node
        .path
        .ends_with("/BrainPanel/BottomLine/BrainDiffToggle/Background/Checkmark")
        && context.app.is_some_and(|app| !app.show_brain_differences)
    {
        return false;
    }
    // `GUIManager.OnTargetChanged`: `OpenBiologyPanelButton` is inactive while the target is an egg.
    if node.path.contains("/BibiteButtons/BiologyButton")
        && context.app.is_some_and(|app| app.selected_egg.is_some())
    {
        return false;
    }
    // `ChallengeManager.objectsToHide`: a challenge hides the pen (dynamic parameters), the placers and the user actions.
    if context.app.is_some_and(App::challenge_is_running)
        && CHALLENGE_HIDDEN
            .iter()
            .any(|tail| node.path.ends_with(tail) || node.path.contains(&format!("{tail}/")))
    {
        return false;
    }
    node.active_in_hierarchy
}

/// The HUD buttons `ChallengeManager.objectsToHide` lists (the Stats panel's Kill and Lay and the pause menu's Save and Load are
/// handled where those are laid out).
const CHALLENGE_HIDDEN: [&str; 3] = [
    "/Layer0/UI Buttons/TopRight/DynamicParametersButton",
    "/Layer0/UI Buttons/BottomLeft/PlacerButton",
    "/Layer0/UI Buttons/BottomLeft/UserActions",
];

/// How far the layout groups the hidden buttons were in move the others (reference px): the 60-wide buttons of the top-right row are
/// 5 apart and right aligned, so the pen's neighbours to its left move right by 65; the bottom-left row is 50 wide, 10 apart and left
/// aligned, so the view button moves left over the two hidden ones.
fn challenge_group_shift(context: &FrameContext<'_>, node: &Node) -> Option<f32> {
    if node.native_rect.is_none() || !context.app.is_some_and(App::challenge_is_running) {
        return None;
    }
    let path = node.path.as_str();
    if path.contains("/Layer0/UI Buttons/BottomLeft/ViewButton") {
        Some(-120.0)
    } else if path.contains("/Layer0/UI Buttons/TopRight/timePanel")
        || path.contains("/Layer0/UI Buttons/TopRight/InformationPanelButton")
    {
        Some(65.0)
    } else {
        None
    }
}

/// The tab buttons are a `HorizontalLayoutGroup` of 60 x 60 buttons 10 apart: when the Biology button is gone the Brain button
/// takes its place.
const BIBITE_BUTTON_STEP: f32 = 70.0;

/// The time-speed slider while its number is typed: `OpenEditPanel` hides `NormalSecton` (the slider) and shows `EditSection`, whose `InputField (TMP)` takes the
/// slider's place; its background is the sliced `InputFieldBackground`, its text `LiberationSans SDF` 20 in dark grey, with the selection highlight or the blinking caret
/// and the `Enter value...` placeholder while empty.
fn emit_time_speed_edit_field(context: &mut FrameContext<'_>, composed: ComposedRect, edit: &crate::value_edit::ValueEdit) {
    let s = context.scale;
    let rect = [composed.pos.x * s, composed.pos.y * s, composed.size.x * s, composed.size.y * s];
    match context.ui.texture_of_alias("InputFieldBackground") {
        Some(texture) => crate::lineage_graph::clip_nine_slice(
            context.frame,
            context.ui,
            texture,
            8.0,
            8.0 * s,
            rect,
            [255, 255, 255, 255],
            [rect[0] - 1.0, rect[1] - 1.0, rect[2] + 2.0, rect[3] + 2.0],
        ),
        None => context.frame.rect(Vec2::new(rect[0], rect[1]), Vec2::new(rect[2], rect[3]), [255, 255, 255, 255]),
    }
    let size = 20.0 * s;
    let text_x = rect[0] + 12.0 * s;
    let middle = rect[1] + rect[3] / 2.0 + 0.5 * s;
    let line = 23.0 * s;
    let before = |chars: usize| crate::value_edit::text_width(&edit.text.chars().take(chars).collect::<String>(), 20.0) * s;
    if let Some((from, to)) = edit.caret.selection() {
        context.frame.rect(
            Vec2::new(text_x + before(from), middle - line / 2.0),
            Vec2::new(before(to) - before(from), line),
            [168, 206, 255, 192],
        );
    }
    let (shown, ink) = if edit.text.is_empty() {
        ("Enter value...", [50, 50, 50, 128])
    } else {
        (edit.text.as_str(), [50, 50, 50, 255])
    };
    let height = size * 1.149;
    context.frame.draws.push(UiDraw::Text {
        text: shown.to_owned(),
        pos: Vec2::new(text_x, middle - height / 2.0),
        px: size,
        color: ink,
        role: FontRole::Body,
        font: Some("LiberationSans SDF".to_owned()),
    });
    if edit.caret_visible() {
        context.frame.rect(
            Vec2::new(text_x + before(edit.caret.pos), middle - line / 2.0),
            Vec2::new(s.max(1.0), line),
            [50, 50, 50, 255],
        );
    }
    // A press on the field keeps it open.
    let id = context.next_hit;
    context.next_hit += 1;
    context.frame.hit(id, Vec2::new(rect[0], rect[1]), Vec2::new(rect[2], rect[3]), UiAction::ValueEditInside);
    context.emitted += 1;
}

fn emit_node_inner(
    context: &mut FrameContext<'_>,
    node: &Node,
    composed: ComposedRect,
    semantics: RectSemantics,
) {
    if !node_shows(context, node) {
        return;
    }
    // `UserSettings.ShowStatusBars`: the health / energy bars over the selected bibite.
    if context.app.is_some_and(|app| !app.show_status_bars)
        && node.path.contains("/Layer0/statusbars")
    {
        return;
    }
    // `DynamicSettingsPanel.onOpen` / `onClose`: `Right Panels/QuickStatsHolder` (the "Info +" button, the Challenge tab, the panels) is inactive while
    // the dynamic settings panel is open.
    if context.app.is_some_and(App::quick_stats_hidden) && node.path.contains("/Right Panels/QuickStatsHolder") {
        return;
    }
    // `InformationPanel.onOpen` / `onClose`: the `OpenButtons` (the Info and challenge tabs) are inactive while the
    // panel is open.
    if context.app.is_some_and(|app| {
        app.panels.is_open(crate::ui::PanelKind::Information)
            || app.panels.is_open(crate::ui::PanelKind::ChallengeTracker)
    }) && node.path.contains("/QuickStatsHolder/OpenButtons")
    {
        return;
    }
    // Native v2 sessions carry final composed 1920-screen bboxes; use them
    // directly so Unity layout groups/canvas scalers are bypassed.
    let composed = if let Some([x0, y0, x1, y1]) = node.native_rect {
        // v2 screen_bbox is in Unity screen space (bottom-left origin).
        ComposedRect {
            pos: Vec2::new(x0 + context.shift[0].unwrap_or(0.0), REFERENCE_HEIGHT - y1 + context.shift[1].unwrap_or(0.0)),
            size: Vec2::new((x1 - x0).max(0.0), (y1 - y0).max(0.0)),
            scale: Vec2::new(1.0, 1.0),
        }
    } else {
        composed
    };
    // The fills of the Biology panel's Flow bar grow from the centre of its track (the centre is 245, the track 24 high at 211.5 from the top).
    let composed = match (
        crate::biology_view::flow_part(&node.path),
        context.app.and_then(|app| app.selection.as_ref()),
        context.app,
    ) {
        (Some(part), Some(panel), Some(app)) if panel.internals.is_some() => {
            use crate::biology_view::FlowPart;
            let scale = crate::biology_view::flow_scale(
                crate::biology_view::selection_seed(app),
                panel.energy_flow,
            );
            let reach = crate::biology_view::flow_reach(panel.energy_flow, scale);
            let (x, w) = match part {
                FlowPart::PositiveFill => (245.0, reach),
                FlowPart::NegativeFill => (245.0 - reach, reach),
                FlowPart::PositiveText => (245.0 - 65.0, 65.0),
                FlowPart::NegativeText => (245.0, 97.6),
            };
            ComposedRect {
                pos: Vec2::new(x, 211.5),
                size: Vec2::new(w, 24.0),
                scale: Vec2::new(1.0, 1.0),
            }
        }
        _ => composed,
    };
    // The fills of the Biology panel's health, energy, growth and age bars follow the selected bibite.
    let composed = match (
        context.app.and_then(|app| app.selection.as_ref()),
        context.app,
    ) {
        (Some(panel), Some(app)) if panel.internals.is_some() => {
            match crate::biology_view::bar_edit(&node.path, panel, crate::biology_view::selection_seed(app)) {
                Some(crate::biology_view::BarEdit::Width(width)) => ComposedRect {
                    size: Vec2::new(width, composed.size.y),
                    ..composed
                },
                Some(crate::biology_view::BarEdit::MarkerX(x)) => ComposedRect {
                    pos: Vec2::new(x, composed.pos.y),
                    ..composed
                },
                None => composed,
            }
        }
        _ => composed,
    };
    // A dead bibite's Biology panel: the Dead Line takes 26.16 above the body section, the hidden Energy row gives 34 back to what is under it.
    let composed = match context.app.and_then(|app| app.selection.as_ref()) {
        Some(panel) if panel.internals.is_some() && panel.dead => {
            use crate::biology_popups::{DEAD_LINE_SHIFT, ENERGY_ROW_PITCH};
            let path = node.path.as_str();
            let rect = |x: f32, y: f32, w: f32, h: f32| ComposedRect {
                pos: Vec2::new(x, y),
                size: Vec2::new(w, h),
                scale: Vec2::new(1.0, 1.0),
            };
            if path.ends_with("/HealthSliderHolder/Title (Dead)") {
                rect(25.0, 144.3 + DEAD_LINE_SHIFT, 90.0, 22.4)
            } else if path.ends_with("/MainSection/Dead Line/Title (1)") {
                rect(25.0, 143.5 + 3.58 - 14.065, 137.65, 28.13)
            } else if path.ends_with("/MainSection/Dead Line/tag") {
                rect(25.0 + 147.65, 143.5 - 14.0, 82.94, 35.16)
            } else {
                let dy = if path.contains("/BodyInfoHolder/HealthSliderHolder") {
                    DEAD_LINE_SHIFT
                } else if path.contains("/BodyInfoHolder/EnergyRateSliderHolder")
                    || path.contains("/BodyInfoHolder/MaturitySliderHolder")
                    || path.contains("/BodyInfoHolder/AgeSliderHolder")
                    || path.contains("/MainSection/InternalInfoHolder")
                {
                    DEAD_LINE_SHIFT - ENERGY_ROW_PITCH
                } else if path.ends_with("/Panels/BibitePanels/BiologyPanel") {
                    // The panel's background gets shorter by what the Energy row gave back less the Dead Line.
                    DEAD_LINE_SHIFT - ENERGY_ROW_PITCH
                } else {
                    0.0
                };
                if path.ends_with("/Panels/BibitePanels/BiologyPanel") {
                    ComposedRect {
                        size: Vec2::new(composed.size.x, composed.size.y + dy),
                        ..composed
                    }
                } else {
                    ComposedRect {
                        pos: Vec2::new(composed.pos.x, composed.pos.y + dy),
                        ..composed
                    }
                }
            }
        }
        _ => composed,
    };
    // `ChallengeManager.objectsToHide` leaves the groups of the hidden buttons to lay themselves out again.
    let composed = match challenge_group_shift(context, node) {
        Some(dx) => ComposedRect {
            pos: Vec2::new(composed.pos.x + dx, composed.pos.y),
            ..composed
        },
        None => composed,
    };
    // The status bars hang at the centre of the screen, as big as the target and the zoom say.
    let composed = match status_bar_placement(context, node) {
        Some(placement) => placement.apply(composed),
        None => composed,
    };
    // The Stats panel's nodes move with the state of its target (a native capture's bounding boxes shift by the layout's change).
    let composed = match context.stats.as_ref().and_then(|stats| {
        crate::stats_states::StatsLayout::rel_of(&node.path).and_then(|rel| stats.shift(rel))
    }) {
        Some([dx, dy, dw, dh]) => ComposedRect {
            pos: Vec2::new(composed.pos.x + dx, composed.pos.y + dy),
            size: Vec2::new(composed.size.x + dw, composed.size.y + dh),
            scale: composed.scale,
        },
        None => composed,
    };
    // The Brain button moves left into the place of the hidden Biology button (a native capture positions it by its box).
    let composed = if node.native_rect.is_some()
        && node.path.contains("/BibiteButtons/BrainButton")
        && context.app.is_some_and(|app| app.selected_egg.is_some())
    {
        ComposedRect {
            pos: Vec2::new(composed.pos.x - BIBITE_BUTTON_STEP, composed.pos.y),
            ..composed
        }
    } else {
        composed
    };
    // The Brain panel grows with the holder of the target's diagram.
    let composed = match context
        .brain
        .as_ref()
        .filter(|_| node.native_rect.is_some())
        .and_then(|brain| {
            node.path
                .split("/BibitePanels/")
                .nth(1)
                .and_then(|rel| brain.shift(rel))
        }) {
        Some([dx, dy, dw, dh]) => ComposedRect {
            pos: Vec2::new(composed.pos.x + dx, composed.pos.y + dy),
            size: Vec2::new(composed.size.x + dw, composed.size.y + dh),
            scale: composed.scale,
        },
        None => composed,
    };
    // What is under the description moves with the lines the description really has.
    let composed = if context.menu.is_some()
        && context.details_shift != 0.0
        && node.native_rect.is_some()
        && (node.path.contains("/Details/StarsDescriptions")
            || node.path.contains("/Details/prevHighscoreSection"))
    {
        ComposedRect {
            pos: Vec2::new(composed.pos.x, composed.pos.y + context.details_shift),
            ..composed
        }
    } else {
        composed
    };
    // The Load list's scrollbar takes 17 from the viewport (`AutoHideAndExpandViewport`): the "Include autosaves" line above the rows loses them
    // on its right (its label is the flexible one, the checkbox and the folder button keep their widths and move left).
    let composed = if let Some(menu) = context.menu
        && menu.view() == crate::menu::MenuView::LoadGame
        && node.native_rect.is_some()
        && menu.list_bar().is_some()
    {
        if node.path.contains("/SettingToggle/Toggle")
            || node.path.contains("/SettingToggle/Button")
        {
            ComposedRect {
                pos: Vec2::new(composed.pos.x - 17.0, composed.pos.y),
                ..composed
            }
        } else if node.path.ends_with("/SettingToggle/Name") {
            ComposedRect {
                size: Vec2::new(composed.size.x - 17.0, composed.size.y),
                ..composed
            }
        } else {
            composed
        }
    } else {
        composed
    };
    if let Some(menu) = context.menu {
        match menu_node_rule(menu, node) {
            MenuRule::Show => {}
            MenuRule::Hide => return,
            MenuRule::ReplaceList => {
                if menu.view() == crate::menu::MenuView::LoadGame {
                    emit_save_list(context, menu);
                } else if menu.view() == crate::menu::MenuView::Library {
                    emit_template_list(context, menu);
                } else {
                    emit_menu_list(context, menu);
                }
                return;
            }
            MenuRule::Preview => {
                emit_menu_preview(context, menu, composed);
                return;
            }
            MenuRule::ListBar => {
                emit_list_bar(context, menu);
                return;
            }
            MenuRule::ChampionSection => {
                let drawer = crate::species_panel::Drawer::new(
                    context.scale,
                    context.ui,
                    context.metrics,
                    menu.cursor(),
                );
                crate::challenge_menu::emit(
                    context.frame,
                    &drawer,
                    menu,
                    [composed.pos.x, composed.pos.y],
                    [composed.size.x, composed.size.y],
                    &mut context.next_hit,
                );
                return;
            }
        }
    }
    // Settings-screen rows: bind to the user settings.
    let saved_row = (context.setting_row, context.toggle_on);
    if let Some(menu) = context.menu
        && menu.view() == crate::menu::MenuView::Settings
        && let Some(label) = settings_row_label(node)
    {
        context.setting_row = crate::user_settings::index_of(&label);
        if let Some(index) = context.setting_row
            && let crate::user_settings::Value::Bool(flag) = menu.user_settings().get(index)
        {
            context.toggle_on = Some(flag);
        }
    }
    let composed = settings_slider_geometry(context, node, composed);
    let composed = speed_slider_geometry(context, node, composed);
    // The number of the time-speed slider is being typed: the slider (`NormalSecton`) is hidden and its field takes its place.
    if node.path.ends_with("/Controls/TopCenter/sliderHolder/BasicSlider/NormalSecton")
        && !crate::dyn_panel::is_generated(&context.session)
        && let Some(edit) = context
            .app
            .and_then(|app| app.value_edit.as_ref())
            .filter(|edit| edit.field == crate::dyn_panel::FieldRef::TimeSpeed)
    {
        emit_time_speed_edit_field(context, composed, edit);
        return;
    }
    let (composed, in_view) = panel_scroll(context, node, composed);
    if in_view != InView::Out
        && context
            .only
            .is_none_or(|only| only.contains(node.path.as_str()))
    {
        // What a mask would cut is drawn only when all of it is in the view (and answers the pointer only there).
        if in_view == InView::Whole {
            emit_images(context, node, composed);
            // The Library's preview: the selected template's bibite over its dark box (`BibitePreview`).
            if let Some(menu) = context.menu
                && menu.view() == crate::menu::MenuView::Library
                && node
                    .path
                    .ends_with("/BibitePreview/Top/BodyPreview/BackGround")
                && let Some(texture) = menu.library_picture()
            {
                context.frame.draws.push(UiDraw::Pixels {
                    texture,
                    pos: composed.pos * context.scale,
                    size: composed.size * context.scale,
                });
                context.emitted += 1;
            }
        }
        emit_texts(context, node, composed);
        if in_view == InView::Whole {
            emit_hit(context, node, composed);
            emit_tip(context, node, composed);
        }
    }

    // The Brain tab's diagram holder is replaced by the target's live brain (`inspector_brain`).
    if node.path.ends_with("/BrainPanel/Body/Holder")
        && context.only.is_none()
        && context.session == "default_world_brain"
        && let Some(app) = context.app
        && crate::inspector_brain::draw(
            context.frame,
            app,
            context.ui,
            context.metrics,
            context.scale,
        )
    {
        return;
    }

    // Dynamic-settings toggle rows: the checkmark follows the live scenario flag.
    let saved_toggle = (context.toggle_on, context.toggle_meta);
    if node.name.starts_with("SettingToggle")
        && !crate::dyn_panel::is_generated(&context.session)
        && let Some(app) = context.app
    {
        let (meta, flag) = live_scenario_flag(app, &active_control_label(node)).unzip();
        context.toggle_on = flag;
        context.toggle_meta = meta;
    }
    if node.name == "Toggle"
        && let Some(index) = context.toggle_meta
        && context.only.is_some()
        && !crate::dyn_panel::is_generated(&context.session)
    {
        let id = context.next_hit;
        context.next_hit += 1;
        let pos = composed.pos * context.scale;
        let size = composed.size * context.scale;
        context
            .frame
            .hit(id, pos, size, UiAction::ToggleScenarioSetting(index as u16));
    }

    let parent_size = composed.size;
    let parent_pivot = Vec2::new(node.rect.pivot.x, node.rect.pivot.y);
    let mut children: Vec<&Node> = node.children.iter().collect();
    children.sort_by_key(|child| child.sibling_index);
    // The in-game settings panel's scroll viewport carries a `Mask`: what lies partly outside it (the rows the layout emits whole) is cut at its edge.
    let cuts = node.name == "Viewport"
        && node.path.contains("/DynamicSettingsPanel/")
        && crate::dyn_panel::is_generated(&context.session);
    if cuts {
        let scale = context.scale;
        context.frame.draws.push(UiDraw::Clip(Some([
            composed.pos.x * scale,
            composed.pos.y * scale,
            composed.size.x * scale,
            composed.size.y * scale,
        ])));
    }
    for child in children {
        if !node_shows(context, child) {
            continue;
        }
        // The Stats panel's rectangles move with the state of its target.
        let moved = context.stats.as_ref().and_then(|stats| {
            crate::stats_states::StatsLayout::rel_of(&child.path)
                .and_then(|rel| stats.rect_of(rel, &child.rect))
        });
        let mut child_rect = moved.unwrap_or(child.rect);
        // The egg section's bars: the fill is as wide as the value says (`ValueSliderHandle.OnValueChange`).
        if let Some(width) = egg_fill_width(context, child, parent_size) {
            child_rect.size_delta.x = width;
        }
        let (size, topdown) = compose_child(&child_rect, parent_size, parent_pivot, semantics);
        let composed_child = ComposedRect {
            pos: Vec2::new(
                composed.pos.x + composed.scale.x * topdown.x,
                composed.pos.y + composed.scale.y * topdown.y,
            ),
            size: Vec2::new(size.x * composed.scale.x, size.y * composed.scale.y),
            scale: Vec2::new(
                composed.scale.x * child.rect.local_scale.x,
                composed.scale.y * child.rect.local_scale.y,
            ),
        };
        emit_node(context, child, composed_child, semantics);
    }
    if cuts {
        context.frame.draws.push(UiDraw::Clip(None));
    }
    // Its number is the edit button (`SettingSlider.OpenEditPanel`): a click turns it into a field to type a speed in.
    if context.app.is_some()
        && !crate::dyn_panel::is_generated(&context.session)
        && node.path.ends_with("/Controls/TopCenter/sliderHolder/BasicSlider/FloatInfoText")
    {
        let id = context.next_hit;
        context.next_hit += 1;
        context.frame.hit(
            id,
            composed.pos * context.scale,
            composed.size * context.scale,
            UiAction::DynEditValue(crate::dyn_panel::FieldRef::TimeSpeed),
        );
    }
    // The time-speed slider's panel is a draggable control (added last so it is on top).
    if context.app.is_some()
        && !crate::dyn_panel::is_generated(&context.session)
        && node
            .path
            .ends_with("/Controls/TopCenter/sliderHolder/BasicSlider/NormalSecton/SliderPanel")
    {
        let id = context.next_hit;
        context.next_hit += 1;
        context.frame.hit(
            id,
            composed.pos * context.scale,
            composed.size * context.scale,
            UiAction::SpeedSlide(845, 1075),
        );
    }
    (context.toggle_on, context.toggle_meta) = saved_toggle;
    (context.setting_row, context.toggle_on) = saved_row;
}

/// Where the status bars are now relative to the capture (`StatusBarsReference.UpdatePlacement`): the object hangs at the centre of the
/// screen (the camera follows the target) with its bottom middle `25 x scale` above it, and `scale = targetScale x 200 / camSize`.
struct StatusBarPlacement {
    /// The scale now over the captured one.
    k: f32,
    /// The pivot (the object's bottom middle) in the capture (reference px, y down).
    from: Vec2,
    /// The pivot now.
    to: Vec2,
}

impl StatusBarPlacement {
    /// A captured rectangle where the bars are now.
    fn apply(&self, rect: ComposedRect) -> ComposedRect {
        ComposedRect {
            pos: self.to + (rect.pos - self.from) * self.k,
            size: rect.size * self.k,
            scale: rect.scale,
        }
    }
}

/// The scale the bars had in the capture (`localScale`; `localPosition.y` was 25 times it).
const STATUS_BARS_CAPTURED_SCALE: f32 = 0.624_463_26;
/// `StatusBarsReference.offset`.
const STATUS_BARS_OFFSET: f32 = 25.0;

/// The placement for a node of the status bars while a bibite or egg is selected; `None` for any other node.
fn status_bar_placement(context: &FrameContext<'_>, node: &Node) -> Option<StatusBarPlacement> {
    if node.native_rect.is_none() || !node.path.contains("/Layer0/statusbars") {
        return None;
    }
    let app = context.app?;
    app.selection.as_ref()?;
    let screen = app.camera.screen;
    let cam_size = crate::render::zoom_to_ortho(app.camera.zoom, screen.y.max(1.0));
    let scale = app.status_bar_scale * 200.0 / cam_size;
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    // The capture's pivot: the screen centre, `25 x 0.6245` above it.
    let centre = context.reference * 0.5;
    let from = Vec2::new(
        centre.x,
        centre.y - STATUS_BARS_OFFSET * STATUS_BARS_CAPTURED_SCALE,
    );
    // The window's own centre in reference px (the canvas fills the screen whatever its shape).
    let window = screen / (2.0 * context.scale.max(1e-3));
    Some(StatusBarPlacement {
        k: scale / STATUS_BARS_CAPTURED_SCALE,
        from,
        to: Vec2::new(window.x, window.y - STATUS_BARS_OFFSET * scale),
    })
}

/// How full a status bar's `imgFilled` is (`Image.fillAmount`, horizontal): `None` for any other image.
fn status_bar_fill(context: &FrameContext<'_>, node: &Node) -> Option<f32> {
    let panel = context.app?.selection.as_ref()?;
    let (health, energy) = crate::stats_values::status_bar_ratios(panel);
    if node.path.ends_with("/statusbars/HealthBar/imgFilled") {
        Some(health)
    } else if node.path.ends_with("/statusbars/EnergyBar/imgFilled") {
        Some(energy)
    } else {
        None
    }
}

/// The width of the `Fill` of one of the egg section's two bars inside its `ValueSlider` of `slider` canvas units
/// (`InfoSliderHandle.ScaledX`); `None` for every other node.
fn egg_fill_width(context: &FrameContext<'_>, node: &Node, slider: Vec2) -> Option<f32> {
    let panel = context.app?.selection.as_ref()?;
    let rest = node.path.split("/BibitePanels/").nth(1)?;
    let (value, scale) = crate::stats_values::egg_slider(panel, rest)?;
    Some(crate::stats_values::slider_fill_width(value, scale, slider))
}

/// A `Selectable` that is not interactable shows its `ColorBlock.disabledColor` (the scenes' buttons
/// keep Unity's default, (0.784, 0.784, 0.784, 0.502)) on its target graphic.
fn disabled_button_tint(tint: Rgba) -> Rgba {
    let scaled = |value: u8, factor: f32| (f32::from(value) * factor).round() as u8;
    [
        scaled(tint[0], 0.784),
        scaled(tint[1], 0.784),
        scaled(tint[2], 0.784),
        scaled(tint[3], 0.502),
    ]
}

fn emit_images(context: &mut FrameContext<'_>, node: &Node, composed: ComposedRect) {
    // The top-centre playback slider's rounded panel background is not visible
    // in the game HUD (verified against the committed save-state HUD overlay);
    // the captured node still carries the semi-transparent Background sprite.
    if node.path.contains("/Controls/TopCenter/") && node.path.ends_with("/SliderPanel") {
        return;
    }
    if node.path.ends_with("/Checkmark") && context.toggle_on == Some(false) {
        return;
    }
    // The challenge list's `ScenariosButton` takes the place of `Challenges Buttons` (`showWithChallenges` / `showWithScenarios`): the same
    // 70 x 70 slot with the `simulation_icon` over the whole button instead of the grey star (which is inset 7.5 on each side).
    if let Some(menu) = context.menu
        && menu.view() == crate::menu::MenuView::NewGame
        && menu.showing_challenges()
        && node
            .path
            .ends_with("/Button holder/Challenges Buttons/icon")
    {
        if let Some(texture) = context.ui.texture_of_alias("simulation_icon") {
            let inset = Vec2::splat(7.5);
            context.frame.draws.push(UiDraw::Image {
                texture,
                pos: (composed.pos - inset) * context.scale,
                size: (composed.size + inset * 2.0) * context.scale,
                tint: [255, 255, 255, 255],
            });
            context.emitted += 1;
        }
        return;
    }
    // View-panel checkmarks follow the live overlay switches (the capture drew
    // every checkmark regardless of the toggle state).
    if node.path.ends_with("/Background/Checkmark")
        && let Some(app) = context.app
        && let Some(rest) = node.path.split("/VisibilityPanel/").nth(1)
    {
        let on = match rest.split('/').next().unwrap_or_default() {
            "Fertility" => app.scene_options.zones,
            "Pheromones" => app.scene_options.pheromones,
            "Ruler" => app.show_ruler,
            "Grid" => app.scene_options.grid,
            "Status bars" => app.show_status_bars,
            "BibiteFOW" => app.scene_options.fov,
            "BibiteTargets" => app.show_targets,
            _ => true,
        };
        if !on {
            return;
        }
    }
    // The Biology panel's organ preview is drawn by the `internals_preview`
    // shader (silhouette + organ portions), which is not available; draw the
    // silhouette sprite and skip the organ sprites that
    // the shader would have composited into it.
    if node.path.contains("/internals preview/") {
        return;
    }
    if node.path.ends_with("/internals preview") {
        // The game's `UI/InternalsPreview` shader, run for the selected bibite's genes (`internals.rs`).
        let params = context
            .app
            .and_then(|app| app.selection.as_ref())
            .and_then(|panel| panel.internals);
        if let Some(params) = params {
            context.frame.draws.push(UiDraw::Pixels {
                texture: crate::internals::preview_cached(&params),
                pos: composed.pos * context.scale,
                size: composed.size * context.scale,
            });
            context.emitted += 1;
            // The food in the stomach and the acid over it (`StomachContainer`, `StomachAcid`).
            if let Some(stomach) = context
                .app
                .and_then(|app| app.selection.as_ref())
                .and_then(|panel| panel.stomach_info.as_ref())
            {
                let seed = context.app.map_or(0, crate::biology_view::selection_seed);
                let running = context.app.is_some_and(|app| !app.paused);
                crate::biology_view::draw_overlays(
                    context.frame,
                    context.ui,
                    composed.pos * context.scale,
                    composed.size * context.scale,
                    &params,
                    stomach,
                    seed,
                    running,
                );
                if let Some(organs) = context.app.and_then(|app| app.selection.as_ref()).and_then(|panel| panel.organs.as_ref()) {
                    crate::biology_view::draw_eggs(context.frame, composed.pos * context.scale, composed.size * context.scale, &params, organs, seed);
                }
            }
        }
        return;
    }
    // Achievements screen: each icon follows the player's unlock state.
    if let Some(menu) = context.menu
        && menu.view() == crate::menu::MenuView::Achievements
        && node.path.ends_with("/Body/AchievementPrefab(Clone)")
        && rect_visible(composed, context.reference)
        && let Some(alias) = achievement_icon_alias(
            bibites_io::prefs::GamePrefs::global(),
            node.sibling_index - 1,
        )
        && let Some(texture) = context.ui.texture_of_alias(&alias)
    {
        context.frame.draws.push(UiDraw::Image {
            texture,
            pos: composed.pos * context.scale,
            size: composed.size * context.scale,
            tint: [255, 255, 255, 255],
        });
        context.emitted += 1;
        return;
    }
    for image in &node.images {
        if image.disabled || !rect_visible(composed, context.reference) {
            continue;
        }
        // A scroll viewport's `UIMask` (white at 5 % alpha) is its `Mask`'s graphic, which the game does not show (a live dump of the dynamic panel has
        // its fill at (38, 38, 38), the `gray_panel`'s, not 11 levels lighter).
        if image.sprite.as_ref().is_some_and(|sprite| sprite.name == "UIMask") {
            continue;
        }
        // A sprite-less `Image` is a flat coloured quad in Unity (the slider's default-value
        // ticks, the zone preview's background); the scroll viewports' images are mask
        // graphics that stay hidden.
        // The target marker of a `ProgressSliderHandle` (the Biology panel's growth bar has it at maturity 1.0, the age bar at the ageing threshold) is a
        // sprite-less (38, 38, 38) image in every session.
        let progress_marker = node.name == "Indicator" && node.path.contains("/ProgressSlider/");
        if image.sprite.is_none()
            && (crate::dyn_panel::is_generated(&context.session) || progress_marker)
            && !matches!(node.name.as_str(), "Viewport" | "Scroll View")
        {
            // A sprite-less `Image` is a flat coloured quad in Unity.
            let tint = image.color.map_or([255, 255, 255, 255], color_to_rgba);
            if tint[3] > 0 {
                context.frame.rect(
                    composed.pos * context.scale,
                    composed.size * context.scale,
                    tint,
                );
                context.emitted += 1;
            }
            continue;
        }
        let Some(texture) = resolve_texture(context.ui, image) else {
            continue;
        };
        let mut tint = image.color.map_or([255, 255, 255, 255], color_to_rgba);
        // Settings colour pickers show the live colour.
        if node.name == "ColorDisplay"
            && let (Some(menu), Some(index)) = (context.menu, context.setting_row)
            && let crate::user_settings::Value::Color(color) = menu.user_settings().get(index)
        {
            tint = color;
        }
        // `deleteTemplateButton.interactable = false` (an official bibite is selected): the button's
        // `ColorBlock.disabledColor`, (0.784, 0.784, 0.784, 0.502), tints its own image.
        if node.name == "Delete Button"
            && let Some(menu) = context.menu
            && menu.view() == crate::menu::MenuView::Library
            && !menu.library_can_delete()
        {
            tint = disabled_button_tint(tint);
        }
        if tint[3] == 0 {
            continue;
        }
        let pos = Vec2::new(
            composed.pos.x * context.scale,
            composed.pos.y * context.scale,
        );
        let size = Vec2::new(
            (composed.size.x * context.scale).max(1.0),
            (composed.size.y * context.scale).max(1.0),
        );
        // A filled image shows the left `fillAmount` of its sprite (the status bars follow the target's health and energy).
        if let Some(fill) = status_bar_fill(context, node) {
            if let Some(picture) = context.ui.texture(texture)
                && fill > 0.0
            {
                context.frame.draws.push(UiDraw::ImageRegion {
                    texture,
                    region: [0.0, 0.0, picture.width as f32 * fill, picture.height as f32],
                    pos,
                    size: Vec2::new(size.x * fill, size.y),
                    tint,
                });
                context.emitted += 1;
            }
            continue;
        }
        let border = image_border(image);
        let sliced =
            image.image_type.eq_ignore_ascii_case("Sliced") && border.iter().any(|b| *b > 0.0);
        if sliced {
            context.frame.draws.push(UiDraw::NineSlice {
                texture,
                border,
                // Sliced borders are drawn at `border * referencePPU / spritePPU` divided by
                // the Image's `pixelsPerUnitMultiplier` (the canvas reference
                // pixels-per-unit is 100). The multiplier is not in the captures; it comes
                // from the game's assets (`image_multipliers.json`) and falls back to the
                // measured per-sprite factor.
                dest_scale: image_multiplier(&node.path).map_or_else(
                    || {
                        image
                            .sprite
                            .as_ref()
                            .map_or(1.0, |sprite| crate::ui::nine_slice_dest_scale(&sprite.name))
                    },
                    |multiplier| 1.0 / multiplier,
                ) * image
                    .sprite_ppu
                    .filter(|ppu| *ppu > 0.0)
                    .map_or(1.0, |ppu| 100.0 / ppu),
                pos,
                size,
                tint,
            });
        } else if let Some(turns) = quarter_turns(node) {
            context.frame.draws.push(UiDraw::ImageTurned {
                texture,
                turns,
                pos,
                size,
                tint,
            });
        } else {
            context.frame.draws.push(UiDraw::Image {
                texture,
                pos,
                size,
                tint,
            });
        }
        context.emitted += 1;
    }
}

/// How many quarter turns counter-clockwise a node's transform turns it about z, when that is a multiple of 90 degrees (`localRotation` of the
/// capture; a spinner's arbitrary angle is not one).
fn quarter_turns(node: &Node) -> Option<u8> {
    let q = node.rect.local_rotation;
    if q.x.abs() > 0.05 || q.y.abs() > 0.05 {
        return None;
    }
    let degrees = (2.0 * q.z.atan2(q.w)).to_degrees().rem_euclid(360.0);
    [(90.0, 1), (180.0, 2), (270.0, 3)]
        .into_iter()
        .find(|(angle, _)| (degrees - angle).abs() < 3.0)
        .map(|(_, turns)| turns)
}

/// Icon sprite of the n-th achievement slot (`AchievementManager.achievements`
/// order, then the four challenges' three stars) for its stored unlock state.
fn achievement_icon_alias(prefs: &bibites_io::prefs::GamePrefs, slot: i32) -> Option<String> {
    const BASE: [&str; 8] = [
        "ACH_BIBITE_EDITOR",
        "ACH_BIBITE_EDITOR_100",
        "ACH_BIBITE_EINSTEIN",
        "ACH_SIM_7DAYS",
        "ACH_SIM_1FPS",
        "ACH_BIBITE_1DAY",
        "ACH_BIBITE_100CHILDREN",
        "ACH_USER_SNAP",
    ];
    const CHALLENGES: [&str; 4] = [
        "APOCALYPSE",
        "LIKERABBITS",
        "DOWNWITHTHEBASICS",
        "THEULTIMATEDUEL",
    ];
    let slot = usize::try_from(slot).ok()?;
    if let Some(id) = BASE.get(slot) {
        let achieved = prefs.flag(id).unwrap_or(false) || crate::achievements::shared_unlocked(id);
        return Some(if slot == 0 {
            format!("ACH_BIBITE_EDITOR_{}", u8::from(achieved))
        } else if achieved {
            "rockstar_placeholder_colored".to_owned()
        } else {
            "rockstar_placeholder_grayscale".to_owned()
        });
    }
    let index = slot - BASE.len();
    let challenge = CHALLENGES.get(index / 3)?;
    let star = index % 3 + 1;
    let key = format!("ACH_{challenge}_{star}STAR");
    let achieved = prefs.flag(&key).unwrap_or(false) || crate::achievements::shared_unlocked(&key);
    Some(format!("ACH_{challenge}_{star}STAR_{}", u8::from(achieved)))
}

fn resolve_texture(ui: &UiAssets, image: &ImageComponent) -> Option<u16> {
    let sprite = image.sprite.as_ref()?;
    // Explicit native sprite overrides (toolbar button-theme PNGs) win over the
    // joined atlas aliases, which are sometimes wrong for icon textures.
    if !sprite.name.is_empty()
        && let Some(texture) = ui.texture_of_alias(&sprite.name)
    {
        return Some(texture);
    }
    // The joined `logical`/`file` fields are sometimes the wrong alias (e.g.
    // `base_circle_icon` joined to Checkmark); the sprite's manifest entry is
    // the authoritative PNG path.
    if let Some(manifest) = &sprite.manifest {
        if !manifest.file.is_empty() {
            if let Some(texture) = ui.texture_of_file(&manifest.file) {
                return Some(texture);
            }
        }
    }
    if let Some(file) = &sprite.file {
        if let Some(texture) = ui.texture_of_file(file) {
            return Some(texture);
        }
    }
    if let Some(logical) = sprite.logical.as_deref() {
        if let Some(texture) = ui.texture_of(logical) {
            return Some(texture);
        }
    }
    if let Some(record) = ui.pick(&[
        sprite.name.as_str(),
        sprite.logical.as_deref().unwrap_or(""),
    ]) {
        return ui.texture_of(&record.logical);
    }
    None
}

/// `Image.pixelsPerUnitMultiplier` of the Image at a captured node path, when it is not 1
/// (`tools/unity/export_image_multipliers.py`): scene objects by their full path, prefab
/// instances (`Name(Clone)`) by `Name/relative/path`.
fn image_multiplier(path: &str) -> Option<f32> {
    use std::collections::HashMap;
    type Tables = (HashMap<String, f32>, HashMap<String, f32>);
    static TABLES: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    let (scene, prefab) = TABLES.get_or_init(|| {
        let candidates = [
            PathBuf::from("assets/game/reference/functional_ui_native/image_multipliers.json"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/game/reference/functional_ui_native/image_multipliers.json"),
        ];
        let value = candidates
            .iter()
            .find_map(|candidate| std::fs::read_to_string(candidate).ok())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
        let table = |key: &str| -> HashMap<String, f32> {
            value
                .as_ref()
                .and_then(|value| value.get(key))
                .and_then(serde_json::Value::as_object)
                .map(|map| {
                    map.iter()
                        .filter_map(|(path, multiplier)| {
                            Some((path.clone(), multiplier.as_f64()? as f32))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        (table("scene"), table("prefab"))
    });
    if let Some(multiplier) = scene.get(path) {
        return Some(*multiplier);
    }
    // The innermost prefab instance owns the node.
    let segments: Vec<&str> = path.split('/').collect();
    let clone = segments
        .iter()
        .rposition(|segment| segment.ends_with("(Clone)"))?;
    let root = segments[clone].trim_end_matches("(Clone)");
    let mut key = root.to_owned();
    for segment in &segments[clone + 1..] {
        key.push('/');
        key.push_str(segment);
    }
    prefab.get(&key).copied()
}

fn image_border(image: &ImageComponent) -> [f32; 4] {
    if let Some(border) = image.runtime_sprite_border {
        return [
            border.x.max(0.0),
            border.y.max(0.0),
            border.z.max(0.0),
            border.w.max(0.0),
        ];
    }
    if let Some(border) = &image.border {
        if border.len() >= 4 {
            return [border[0], border[1], border[2], border[3]];
        }
    }
    if let Some(sprite) = &image.sprite {
        if let Some(border) = &sprite.border {
            if border.len() >= 4 {
                return [border[0], border[1], border[2], border[3]];
            }
        }
    }
    [0.0; 4]
}

/// TMP SmallCaps runs: capitals keep the font size, lowercase letters are
/// drawn as uppercase glyphs at `capsMultiplier` (native title measures ~0.79 of
/// cap height; TMP's default is 0.8).
fn small_caps_runs(line: &str, px: f32) -> Vec<(String, f32)> {
    let small_px = px * 0.8;
    let mut runs: Vec<(String, f32)> = Vec::new();
    let mut current = String::new();
    let mut current_px = px;
    for character in line.chars() {
        if character.is_whitespace() {
            if !current.is_empty() {
                runs.push((std::mem::take(&mut current), current_px));
            }
            runs.push((" ".to_owned(), 0.0));
            continue;
        }
        let target_px = if character.is_lowercase() {
            small_px
        } else {
            px
        };
        if !current.is_empty() && (target_px - current_px).abs() > 0.01 {
            runs.push((std::mem::take(&mut current), current_px));
        }
        current_px = target_px;
        for upper in character.to_uppercase() {
            current.push(upper);
        }
    }
    if !current.is_empty() {
        runs.push((current, current_px));
    }
    runs
}

fn emit_texts(context: &mut FrameContext<'_>, node: &Node, composed: ComposedRect) {
    if node.path.ends_with("/geneName") {
        // A new gene row starts: forget the previous row's gene so a row whose
        // name is clipped can never show the previous gene's value.
        context.last_gene = None;
    }
    if node.path.ends_with("/Ruler/FloatInfoText") {
        // Ruler text is procedural (see emit_ruler); the dump's value is stale.
        return;
    }
    for text in &node.texts {
        if !rect_visible(composed, context.reference) {
            continue;
        }
        let content = if let Some(menu) = context.menu {
            settings_live_text(menu, context.setting_row, node, &text.text)
                .unwrap_or_else(|| menu_live_text(menu, node, &text.text))
        } else {
            context.app.map_or_else(
                || text.text.clone(),
                |app| live_text(app, node, text, &mut context.last_gene),
            )
        };
        if content.is_empty() {
            continue;
        }
        let content = if text.rich_text {
            strip_rich_text(&content)
        } else {
            content
        };
        let style = format!("{} {}", text.font_style, text.text_style).to_ascii_lowercase();
        let small_caps = style.contains("smallcaps");
        let content = if !small_caps && style.contains("uppercase") {
            content.to_uppercase()
        } else {
            content
        };
        let outline_material_label = text
            .font
            .as_ref()
            .is_some_and(|font| font.name.eq_ignore_ascii_case("LiberationSans SDF"))
            && text
                .color
                .is_some_and(|c| c.r > 0.9 && c.g > 0.9 && c.b > 0.9)
            && (node.path.contains("/BibitePanels/")
                || node.path.contains("/SelectionPanel/")
                || node.path.contains("/Popuholder/"));
        // TMP "LiberationSans SDF" with the Bold style: the asset has no bold face, so TMP widens the glyphs of the regular atlas and
        // advances every character 0.07 em more (`TMP_FontAsset.boldSpacing`): drawn here by the same atlas with the same two changes
        // (the Roboto Bold stand-in of the first replay was 12-15 % too narrow). The material's face colour is black.
        let liberation = text
            .font
            .as_ref()
            .is_some_and(|font| font.name.eq_ignore_ascii_case("LiberationSans SDF"));
        let heavy_label = outline_material_label || (style.contains("bold") && liberation);
        let role = if heavy_label {
            FontRole::Bold
        } else {
            font_role(text)
        };
        let tmp_bold_name = (heavy_label && liberation).then(|| {
            if style.contains("bold") {
                format!("LiberationSans SDF{}", crate::text_sdf::BOLD_SUFFIX)
            } else {
                "LiberationSans SDF".to_owned()
            }
        });
        // The Brain panel's species name (`SpeciesHolder/SpeciesText`): `Roboto-Regular SDF`, bold style (TMP widens the regular atlas), black.
        // (The Stats panel's "Tag:" and its tag are the same: `Roboto-Regular SDF`, Bold, black.)
        let species_text = node.path.ends_with("/SpeciesHolder/SpeciesText")
            || node.path.contains("/StatsPanel/MainSection/TagHolder/");
        let tmp_bold_name = if species_text {
            Some(format!("Roboto-Regular SDF{}", crate::text_sdf::BOLD_SUFFIX))
        } else {
            tmp_bold_name
        };
        // A character spacing (only the inspector's gene lines have one: -5 hundredths of an em, on the value in the Bold style, which TMP draws with the
        // regular atlas widened and 0.07 em more per advance): it goes into the font name, so that the placement and the drawing advance alike.
        let spaced = text.character_spacing != 0.0 && !heavy_label && !species_text;
        let tmp_bold_name = if spaced {
            let base = text.font.as_ref().map_or("", |font| font.name.as_str());
            let bold = if style.contains("bold") && !liberation {
                crate::text_sdf::BOLD_SUFFIX
            } else {
                ""
            };
            Some(format!(
                "{base}{bold}{}{}",
                crate::text_sdf::SPACING_MARK,
                (text.character_spacing * 10.0).round() as i32
            ))
        } else {
            tmp_bold_name
        };
        // Any other TMP text in the Bold style (`FontStyles.Bold`) on an asset that has no bold typeface of its own - the game's assets have none -
        // is drawn by the same synthetic bold: the regular atlas widened and 0.07 em more per advance. The version label of the title screen
        // (Boston Black, 28, Bold) was 14 % narrower than the game's without it.
        let tmp_bold_name = if tmp_bold_name.is_none() && style.contains("bold") {
            text.font
                .as_ref()
                .map(|font| format!("{}{}", font.name, crate::text_sdf::BOLD_SUFFIX))
        } else {
            tmp_bold_name
        };
        let px = (text.font_size.max(1.0) * context.scale * context.local_scale).max(6.0);
        // The game draws the time block dark on the light time panel even
        // though the captured TMP colour is white (its outline material).
        let color = if node.path.ends_with("/timeText") || outline_material_label {
            // The "LiberationSans SDF - Outline" material renders these white TMP
            // labels dark on the light inspector panels (native reference: pure black).
            [0, 0, 0, 255]
        } else {
            text.color.map_or([236, 236, 240, 255], color_to_rgba)
        };
        // The `LiberationSans SDF` material's face colour is black and the shader multiplies it by the vertex colour: the text is black
        // whatever colour the label carries (only its alpha counts).
        let color = if liberation || species_text { [0, 0, 0, color[3]] } else { color };
        if color[3] == 0 {
            continue;
        }
        // TMP margins apply to every text. The capture only keeps the horizontal alignment
        // (`Left`/`Center`/`Right`), so the vertical one is inferred: the generated panels'
        // labels are middle-aligned, and so is a label in a box that is at most about three
        // lines tall; a taller box is a paragraph that starts at the top.
        let generated = crate::dyn_panel::is_generated(&context.session);
        let margin = text.margin.map_or([0.0; 4], |m| [m.x, m.y, m.z, m.w]);
        let left = (composed.pos.x + margin[0]) * context.scale;
        let top = (composed.pos.y + margin[1]) * context.scale;
        let width = ((composed.size.x - margin[0] - margin[2]) * context.scale).max(0.0);
        let height = ((composed.size.y - margin[1] - margin[3]) * context.scale).max(0.0);
        // The load panel's size value box is auto-sized in the game; keep it left
        // aligned so long values do not run over the "Sim Size:" title.
        let alignment = if context.menu.is_some() && node.path.ends_with("/Size/FloatInfoText") {
            "left".to_owned()
        } else {
            text.alignment.to_ascii_lowercase()
        };
        let line_height = context.metrics.map_or(px * 1.25, |metrics| {
            metrics.line_height_named(
                tmp_bold_name.as_deref().or_else(|| text.font.as_ref().map(|font| font.name.as_str())),
                role,
                px,
            )
        }) + text.line_spacing * px / 100.0;
        // Menu panels wrap long descriptions inside their text box (`TMP wordWrap`).
        let wrapped;
        // The paragraphs of the Additional Tools and Credits tabs (`desc`, and the `title` of a section that is a sentence) and the text beside the title
        // scene's Patreon button wrap at their box, as it is (the others lose 20 more); the section titles' boxes are as wide as their text.
        let in_tab = (node.path.contains("/Tools Tab/Viewport/Content/")
            || node.path.contains("/CreditsTab/Viewport/Content/"))
            && (node.name == "desc" || node.name == "title");
        // (The title menu itself is replayed without a menu state: the Patreon note wraps there too.)
        let patreon_note = node.path.ends_with("/Patreon Holder/Button/Text (TMP)");
        let tab_paragraph = in_tab || patreon_note;
        let content_ref: &str = if text.word_wrap
            && (patreon_note
                || (context.menu.is_some()
                    && (node.path.ends_with("/Details/Description")
                        || node.path.ends_with("/BibitePreview/Desc")
                        || node.path.contains("/ParametersPanel/Content/Options/")
                        || in_tab)))
            && width > 0.0
        {
            let measure = |line: &str| -> f32 {
                context
                    .metrics
                    .map_or(line.chars().count() as f32 * px * 0.52, |metrics| {
                        metrics.measure_named(
                            tmp_bold_name.as_deref().or_else(|| text.font.as_ref().map(|font| font.name.as_str())),
                            role,
                            line,
                            px,
                        )
                    })
            };
            let slack = if tab_paragraph { 0.0 } else { 20.0 };
            let limit = (width - slack * context.scale).max(1.0);
            let mut made = wrap_words(&content, limit, &measure);
            if tab_paragraph && line_height > 0.0 {
                // The capture's box is as high as the lines the game made of the text: where our measure of it is a hair wider than the game's, the
                // line it adds is the measure's (the sentence of the Credits' Patreon section ends with a "<3" that fits by a pixel).
                let game_lines = (composed.size.y * context.scale / line_height)
                    .round()
                    .max(1.0) as usize;
                for step in 1..=3 {
                    if made.split('\n').count() <= game_lines {
                        break;
                    }
                    made = wrap_words(&content, limit * (1.0 + 0.005 * step as f32), &measure);
                }
            }
            wrapped = made;
            &wrapped
        } else {
            &content
        };
        let lines: Vec<&str> = content_ref.split('\n').collect();
        // The `Details` of the New Game screen is laid out again for the description it shows: the lines it has beyond (or short of)
        // the captured text's move what is under it (`StarsDescriptions`, `prevHighscoreSection`).
        if context.menu.is_some()
            && node.path.ends_with("/Details/Description")
            && node.native_rect.is_some()
            && line_height > 0.0
        {
            let captured = (composed.size.y * context.scale / line_height)
                .round()
                .max(1.0);
            context.details_shift = (lines.len() as f32 - captured) * line_height / context.scale;
        }
        let total_height = line_height * lines.len() as f32;
        // TMP centres a line on its glyphs' ascent-to-descent extent, which is the line height less the font's line gap: the labels drawn
        // with the Liberation atlas sit that much lower than a box centred on the line height would put them.
        let centring_gap = tmp_bold_name
            .as_deref()
            .and_then(|name| context.metrics.map(|metrics| metrics.line_gap_named(Some(name), role, px)))
            .unwrap_or(0.0);
        let mut y = top;
        // Generated panels centre their labels, but a paragraph box (a description field) stays
        // top-aligned.
        // (The New Game screen's modifier names are middle aligned in boxes up to 123 high, as the game draws them beside their dropdowns.)
        let option_name = node.path.contains("/ParametersPanel/Content/Options/") && node.name == "Name";
        let alignment = if (height <= if generated { 6.0 } else { 3.0 } * line_height || option_name)
            && !alignment.contains("top")
            && !alignment.contains("bottom")
        {
            format!("middle {alignment}")
        } else {
            alignment
        };
        if alignment.contains("middle") {
            // A single line taller than its box is centred on it all the same (TMP overflows both ways); a paragraph starts at the top.
            let offset = (height - total_height + centring_gap) * 0.5;
            y += if lines.len() == 1 {
                offset
            } else {
                offset.max(0.0)
            };
        } else if alignment.contains("bottom") {
            y += (height - total_height).max(0.0);
        }
        // The software text path draws one line per `UiDraw::Text` (it ignores
        // embedded newlines), so split TMP's multi-line labels explicitly.
        for (index, line) in lines.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let measure_run = |run: &str, run_px: f32| -> f32 {
                context
                    .metrics
                    .map_or(run.chars().count() as f32 * run_px * 0.52, |metrics| {
                        metrics.measure_named(
                            tmp_bold_name.as_deref().or_else(|| text.font.as_ref().map(|font| font.name.as_str())),
                            role,
                            run,
                            run_px,
                        )
                    })
            };
            let estimated = if small_caps {
                // The runs drawn below: capitals at the size, lowercase letters as capitals at 0.8 of it, a space 0.32 of the size.
                small_caps_runs(line, px)
                    .iter()
                    .map(|(run, run_px)| {
                        if run == " " {
                            px * 0.32
                        } else {
                            measure_run(run, *run_px)
                        }
                    })
                    .sum::<f32>()
            } else {
                measure_run(line, px)
            } + if spaced {
                0.0
            } else {
                text.character_spacing * line.chars().count() as f32 * px / 100.0
            };
            let mut x = left;
            // TMP leaves a line's trailing spaces out of its width when it centres or right-aligns it (a button's "Sort " is centred as "Sort").
            let aligned = if small_caps || !line.ends_with(' ') {
                estimated
            } else {
                estimated - (measure_run(line, px) - measure_run(line.trim_end_matches(' '), px))
            };
            if alignment.contains("center") {
                x += (width - aligned) * 0.5;
            } else if alignment.contains("right") {
                x += width - aligned;
            }
            let line_pos = Vec2::new(x, y + line_height * index as f32);
            // A scrolling panel's mask cuts the text: only the lines inside its view are there.
            if let Some([view_top, view_bottom]) = context.scroll_clip
                && (line_pos.y < view_top * context.scale - 0.5
                    || line_pos.y + line_height > view_bottom * context.scale + 0.5)
            {
                continue;
            }
            let font_name = if heavy_label || species_text || spaced || tmp_bold_name.is_some() {
                Some(tmp_bold_name.clone().unwrap_or_else(|| "Roboto-Bold SDF NoOutline".to_owned()))
            } else {
                text.font.as_ref().map(|font| font.name.clone())
            };
            if small_caps {
                let mut run_x = line_pos.x;
                for (run, run_px) in small_caps_runs(line, px) {
                    if run == " " {
                        run_x += px * 0.32;
                        continue;
                    }
                    let run_width = measure_run(&run, run_px);
                    context.frame.draws.push(UiDraw::Text {
                        text: run,
                        // Keep the Common baseline: smaller caps sit lower.
                        pos: Vec2::new(run_x, line_pos.y + (px - run_px) * 0.95),
                        px: run_px,
                        color,
                        role,
                        font: font_name.clone(),
                    });
                    run_x += run_width;
                    context.emitted += 1;
                }
            } else {
                context.frame.draws.push(UiDraw::Text {
                    text: (*line).to_owned(),
                    pos: line_pos,
                    px,
                    color,
                    role,
                    font: font_name,
                });
                context.emitted += 1;
            }
        }
    }
}

/// Removes TMP rich-text tags (`<b>`, `<color=…>`, …); the viewer draws plain
/// glyphs, so literal tag text would leak into the HUD. What is not a tag stays as it is, as in TMP ("I truly can't thank you enough <3").
fn strip_rich_text(text: &str) -> String {
    let mut stripped = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        stripped.push_str(&rest[..start]);
        let tag = &rest[start + 1..];
        // A tag starts with a letter, `/` or `#` and ends at the next `>`.
        let is_tag = tag
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '/' || first == '#');
        match tag.find('>').filter(|_| is_tag) {
            Some(end) => rest = &tag[end + 1..],
            None => {
                stripped.push('<');
                rest = tag;
            }
        }
    }
    stripped.push_str(rest);
    stripped
}

/// Live values for the captured native inspector sessions. The node paths and
/// the value formats (`15.61 u`, `124.79 u2`, `3.15 g`, `0.0HP`, `%.3f` genes)
/// come from the game capture; the numbers come from the selected bibite.
fn live_inspector_text(
    app: &App,
    path: &str,
    text: &TextComponent,
    last_gene: &mut Option<usize>,
) -> Option<String> {
    let panel = app.selection.as_ref()?;
    // Status bars above the world (`StatusBarsReference`): a bibite's health and energy, an egg's progress and free energy.
    if path.ends_with("/statusbars/HealthBar/Text (TMP)") {
        return Some(crate::stats_values::status_bar_texts(panel).0);
    }
    if path.ends_with("/statusbars/EnergyBar/Text (TMP)") {
        return Some(crate::stats_values::status_bar_texts(panel).1);
    }
    let rest = path.split("/BibitePanels/").nth(1)?;
    // The numbers of the Stats panel's rows, in the format of their `FloatValueTextHandle`s.
    if let Some(value) = crate::stats_values::text(panel, rest) {
        return Some(value);
    }
    let value = match rest {
        // `BibiteStatsPanel.FillPanel`: "None" without a species.
        "StatsPanel/MainSection/SpeciesHolder/SpeciesText" if panel.species_id.is_none() => {
            "None".to_owned()
        }
        "StatsPanel/MainSection/SpeciesHolder/SpeciesText"
        | "GenesPanel/SpeciesHolder/SpeciesText"
        | "BrainPanel/Top Section/First Row/SpeciesHolder/SpeciesText" => {
            panel.species_name.clone()
        }
        // While typing, the field shows the draft with a caret.
        "StatsPanel/MainSection/TagHolder/Label Holder/TagLabel" if app.tag_edit.is_some() => {
            crate::text_field::with_caret(app.tag_edit.as_deref().unwrap_or_default())
        }
        "StatsPanel/MainSection/TagHolder/Label Holder/TagLabel" => panel
            .species_tag
            .clone()
            .unwrap_or_else(|| "None".to_owned()),
        // Biology panel.
        r if r.ends_with("/HealthSliderHolder/ValueSlider/ValueHolder/HealthValue") => {
            format!("{:.1}", panel.health)
        }
        r if r.ends_with("/HealthSliderHolder/ValueSlider/ValueHolder/MaxHealthValue") => {
            format!("{:.1}HP", panel.max_health)
        }
        r if r.ends_with("/EnergySliderHolder/ValueSlider/ValueHolder/EnergyValue") => {
            format!("{:.1}", panel.energy)
        }
        r if r.ends_with("/EnergySliderHolder/ValueSlider/ValueHolder/MaxEnergyValue") => {
            format!("{:.1}E", panel.max_energy)
        }
        // Species distance (`FloatValueFormat` percentage with a forced sign).
        "GenesPanel/SpeciesHolder/Distance Group/FloatInfoText" => {
            format!("{:+.0}%", panel.gene_distance? * 100.0)
        }
        "BrainPanel/Top Section/First Row/SpeciesHolder/Distance Group/FloatInfoText" => {
            format!("{:+.0}%", panel.brain_distance? * 100.0)
        }
        // Brain panel header.
        // `FillBrainPanel`: the nodes the diagram draws (`usedIndexes.Count`) and every connection.
        "BrainPanel/Top Section/Info Row/Nodes/FloatInfoText" => app
            .inspector_brain_counts()
            .map_or(panel.brain_nodes as usize, |(nodes, _)| nodes)
            .to_string(),
        "BrainPanel/Top Section/Info Row/Connections/FloatInfoText" => app
            .inspector_brain_counts()
            .map_or(panel.brain_synapses as usize, |(_, synapses)| synapses)
            .to_string(),
        // Genes panel: `geneName` identifies the row, `value` is `%.3f`, and the
        // species-difference column is cleared (the viewer has no species diff).
        r if r.starts_with("GenesPanel/Body/GenesHolder/GeneLine(Clone)/") => {
            let field = r.rsplit('/').next().unwrap_or("");
            match field {
                "geneName" => {
                    let name = text.text.trim().trim_end_matches(':').trim();
                    *last_gene = bibites_core::genome::GENE_DISPLAY_NAMES
                        .iter()
                        .position(|candidate| *candidate == name);
                    return None;
                }
                "value" => format!("{:.3}", panel.genes[(*last_gene)?]),
                // `GeneLineInfo.SetGene`: empty when equal to the species gene,
                // otherwise the signed flat difference as "(+0.017)".
                "difference" => match (*last_gene, panel.species_genes) {
                    (Some(index), Some(species)) => {
                        let delta = panel.genes[index] - species[index];
                        let scale = panel.genes[index].abs().max(species[index].abs()).max(1.0);
                        if delta.abs() <= 1e-6 * scale {
                            String::new()
                        } else {
                            format!("({delta:+.3})")
                        }
                    }
                    _ => String::new(),
                },
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(value)
}

/// Replaces volatile captured text for nodes whose live value the viewer knows.
fn live_text(
    app: &App,
    node: &Node,
    text: &TextComponent,
    last_gene: &mut Option<usize>,
) -> String {
    let path = node.path.as_str();
    if let Some(live) = live_inspector_text(app, path, text, last_gene) {
        return live;
    }
    if crate::biology_view::flow_part(path).is_some()
        && let Some(panel) = app.selection.as_ref()
        && panel.internals.is_some()
    {
        return crate::biology_view::flow_text(panel.energy_flow);
    }
    if path.ends_with("/timeText") {
        // `GUIManager.UpdateTime`: the numbers are the `TimeKeeper`'s measurements (the ticks and frames that really ran
        // in the last half second), which stay as they were while the game is paused.
        return crate::time_keeper::time_panel_text(app.simulated_time(), &app.time_keeper);
    }
    if path.contains("sliderHolder/BasicSlider/FloatInfoText") {
        // While the number is being typed, the text shows what the field means (`SettingSlider.UpdateValueDuringEdit`).
        if let Some(edit) = app.value_edit.as_ref().filter(|edit| edit.field == crate::dyn_panel::FieldRef::TimeSpeed)
            && let Some((rules, _)) = app.typed_rules(edit.field)
            && let Some(value) = rules.preview(&edit.text)
        {
            return format!("X {value:.2}");
        }
        // The captured label is already the game's paused value ("X 0.00").
        return if app.paused {
            text.text.clone()
        } else {
            format!("X {:.2}", app.speed)
        };
    }
    text.text.clone()
}

fn font_role(text: &TextComponent) -> FontRole {
    if let Some(font) = &text.font {
        let hint = format!(
            "{} {} {}",
            font.role.as_deref().unwrap_or(""),
            font.name,
            text.font_style
        )
        .to_ascii_lowercase();
        if hint.contains("mono") || hint.contains("inconsolata") {
            return FontRole::Mono;
        }
        if hint.contains("bold") {
            return FontRole::Bold;
        }
    }
    FontRole::Body
}

fn rect_visible(rect: ComposedRect, reference: Vec2) -> bool {
    let margin = 64.0;
    rect.pos.x + rect.size.x >= -margin
        && rect.pos.y + rect.size.y >= -margin
        && rect.pos.x <= reference.x + margin
        && rect.pos.y <= reference.y + margin
}

/// A captured control with a `TooltipTrigger` gets a hover rectangle: the pointer resting on it
/// shows its tooltip (`crate::tooltip`).
fn emit_tip(context: &mut FrameContext<'_>, node: &Node, composed: ComposedRect) {
    // The name label of a Settings-screen row shows the setting's name and help text
    // (`UpdateText(Name, HelperText)`; the scene holds no trigger for these generated rows).
    if node.name == "Name"
        && context.menu.is_some()
        && let Some(index) = context.setting_row
        && let Some(tip) = crate::tooltip::of_user_setting(crate::user_settings::DEFS[index].label)
    {
        context.frame.tips.push(crate::tooltip::TipRect {
            pos: composed.pos * context.scale,
            size: composed.size * context.scale,
            tip,
        });
        return;
    }
    // In the challenge list the two buttons beside it are `ScenariosButton` and `New Challenge Button`: their tooltips are the ones shown.
    let swapped;
    let challenge_list = context.menu.is_some_and(|menu| {
        menu.view() == crate::menu::MenuView::NewGame && menu.showing_challenges()
    });
    let path = if challenge_list
        && (node.path.ends_with("/Button holder/Challenges Buttons")
            || node.path.ends_with("/Button holder/New Scenario Button"))
    {
        swapped = node
            .path
            .replace("/Challenges Buttons", "/ScenariosButton")
            .replace("/New Scenario Button", "/New Challenge Button");
        swapped.as_str()
    } else {
        node.path.as_str()
    };
    let Some(tip) = crate::tooltip::find(path) else {
        return;
    };
    context.frame.tips.push(crate::tooltip::TipRect {
        pos: composed.pos * context.scale,
        size: composed.size * context.scale,
        tip,
    });
}

fn emit_hit(context: &mut FrameContext<'_>, node: &Node, composed: ComposedRect) {
    if let Some(menu) = context.menu {
        // Native menu panels: only the commands the menu knows are interactive.
        let row_action = context.setting_row.and_then(|index| {
            let name = node.name.as_str();
            let x0 = (composed.pos.x * context.scale).round().max(0.0) as u16;
            let x1 = ((composed.pos.x + composed.size.x) * context.scale)
                .round()
                .max(0.0) as u16;
            // A dropdown row opens its list under the control.
            if name == "Dropdown" {
                return Some(UiAction::DropdownOpen(crate::ui::DropdownTarget::Setting(
                    index as u16,
                )));
            }
            let command = match name {
                "Toggle" => crate::ui::MenuCommand::SettingToggle(index as u16),
                "ResetButton" => crate::ui::MenuCommand::SettingReset(index as u16),
                "Slider" => crate::ui::MenuCommand::SettingSlide(index as u16, x0, x1),
                "ColorDisplay" => crate::ui::MenuCommand::Unavailable(4),
                _ => return None,
            };
            Some(UiAction::Menu(command))
        });
        let Some(action) = row_action.or_else(|| menu_hit_action(menu, node)) else {
            return;
        };
        let id = context.next_hit;
        context.next_hit += 1;
        let pos = composed.pos * context.scale;
        let size = Vec2::new(
            (composed.size.x * context.scale).max(1.0),
            (composed.size.y * context.scale).max(1.0),
        );
        context.frame.hit(id, pos, size, action);
        return;
    }
    let synthetic_dropdown = node.name == "Dropdown"
        && known_dropdown_options(&node.path, &active_control_label(node)).is_some();
    if !node.controls.iter().any(is_interactable_control) && !synthetic_dropdown {
        return;
    }
    // Known controls keep their existing action; every other active
    // interactable control emits a stable id so the framework can bind it
    // without another dump/name allow-list. `App` currently logs
    // `ActivateControl` as a no-op until the panel/action registry is filled.
    let action = control_action(&context.session, node).unwrap_or(UiAction::ActivateControl(
        control_id(&context.session, &node.path),
    ));
    let id = context.next_hit;
    context.next_hit += 1;
    let pos = Vec2::new(
        composed.pos.x * context.scale,
        composed.pos.y * context.scale,
    );
    let size = Vec2::new(
        (composed.size.x * context.scale).max(1.0),
        (composed.size.y * context.scale).max(1.0),
    );
    context.frame.hit(id, pos, size, action);
}

fn functional_action(session: &str, node: &Node, label: &str) -> Option<UiAction> {
    let _ = session;
    let path = node.path.to_ascii_lowercase();
    let label_lower = label.to_ascii_lowercase();
    let name = node.name.as_str();

    // Welcome/title menu rows, also active inside functional subviews.
    if path.contains("/buttonspanel/") {
        return match name {
            "Continue" => Some(UiAction::Invoke(ControlCommand::QuickLoad)),
            "NewSim" => Some(UiAction::Invoke(ControlCommand::OpenScenarioPicker)),
            "LoadSim" => Some(UiAction::Invoke(ControlCommand::OpenLoadPanel)),
            "Editor" => Some(UiAction::OpenPanel(PanelKind::Library)),
            "Settings" => Some(UiAction::OpenPanel(PanelKind::Settings)),
            "Achievements" => Some(UiAction::OpenPanel(PanelKind::Achievements)),
            "More" => Some(UiAction::OpenPanel(PanelKind::Tools)),
            "Credits" => Some(UiAction::OpenPanel(PanelKind::Credits)),
            "Quit" => Some(UiAction::Quit),
            _ => None,
        };
    }

    // Settings pages/selectors.
    if path.contains("/settingspanel/") {
        return match name {
            "CloseButton" => Some(UiAction::ClosePanel),
            "GraphicsButton" | "Gameplay Button" | "Controls Button" => {
                Some(UiAction::Invoke(ControlCommand::OpenSettings))
            }
            "Reset Warnings" => Some(UiAction::Invoke(ControlCommand::ResetWarnings)),
            "ResetButton" => Some(UiAction::Invoke(ControlCommand::ResetSettings)),
            _ => None,
        };
    }

    // Load-game panel.
    if path.contains("/loadgamepanel/") {
        return match name {
            "CloseButton" => Some(UiAction::ClosePanel),
            "LoadButton" => Some(UiAction::Invoke(ControlCommand::LoadSelectedSave)),
            _ => None,
        };
    }

    // New-game scenario picker / creation options.
    if path.contains("/newgamescenarios/") {
        if name == "Return" || label_lower.contains("return to menu") {
            return Some(UiAction::ClosePanel);
        }
        if label_lower.contains("launch simulation")
            || label_lower.contains("try a challenge")
            || label_lower.contains("engineer a bibite")
            || name == "New Scenario Button"
        {
            return Some(UiAction::OpenPanel(PanelKind::ScenarioPicker));
        }
        if name == "StartButton" || label_lower.contains("start simulation") {
            return Some(UiAction::Invoke(ControlCommand::OpenScenarioPicker));
        }
        return None;
    }

    // Functional welcome sub-panels.
    if name == "BackButton" && path.contains("achievementspanel") {
        return Some(UiAction::ClosePanel);
    }
    if name == "CloseButton" {
        return Some(UiAction::ClosePanel);
    }
    if label_lower.contains("bibliothèque") || label_lower.contains("library") || name == "Editor"
    {
        return Some(UiAction::OpenPanel(PanelKind::Library));
    }
    if name == "Tools Tab" || label_lower.contains("additional tools") {
        return Some(UiAction::OpenPanel(PanelKind::Tools));
    }
    if label_lower.contains("achievements") {
        return Some(UiAction::OpenPanel(PanelKind::Achievements));
    }
    if label_lower.contains("credits") || path.contains("/creditspanel/") {
        return Some(UiAction::OpenPanel(PanelKind::Credits));
    }

    // In-world toolbar/tool panels and view toggles.
    match name {
        // Gear = the user settings (Graphics / Game / Hotkeys); pen = the dynamic
        // (scenario) parameters panel.
        "SettingsButton" => return Some(UiAction::OpenPanel(PanelKind::Settings)),
        "DynamicParametersButton" => return Some(UiAction::OpenPanel(PanelKind::WorldDynamics)),
        "AncestralLineageButton" => return Some(UiAction::OpenPanel(PanelKind::Lineage)),
        "OpenGenes" => return Some(UiAction::SetTab(InspectorTab::Genes)),
        "ExitButton" => return Some(UiAction::ClosePanel),
        "PlacePellets" => {
            return Some(UiAction::Invoke(ControlCommand::SelectTool(
                ToolKind::Pellet,
            )));
        }
        // The three placer buttons make three different things (`WorldObjectsSpawner`).
        "PheroTower" => {
            return Some(UiAction::Invoke(ControlCommand::SelectTool(
                ToolKind::PheroTower,
            )));
        }
        "RadTower" => {
            return Some(UiAction::Invoke(ControlCommand::SelectTool(
                ToolKind::Tower,
            )));
        }
        "ColorSelector" => {
            return Some(UiAction::Invoke(ControlCommand::SelectTool(
                ToolKind::Killer,
            )));
        }
        // `BibitePlacer.ButtonFromTemplatePressed`.
        "LoadBibite" => return Some(UiAction::Invoke(ControlCommand::LoadBibite)),
        "KillHalf" => return Some(UiAction::Invoke(ControlCommand::RequestKillHalf)),
        "RemovePellets" => {
            return Some(UiAction::Invoke(ControlCommand::RequestRemoveHalfPellets));
        }
        "GridToggle" => return Some(UiAction::ToggleOverlay(OverlayKind::Grid)),
        "RulerToggle" => return Some(UiAction::ToggleOverlay(OverlayKind::Ruler)),
        "bars toggle" => return Some(UiAction::ToggleOverlay(OverlayKind::StatusBars)),
        "FOW toggle" => return Some(UiAction::ToggleOverlay(OverlayKind::FieldOfView)),
        "Targets toggle" => return Some(UiAction::ToggleOverlay(OverlayKind::Targets)),
        _ => {}
    }
    if path.contains("/visibilitypanel/pheromones/toggle") {
        return Some(UiAction::ToggleOverlay(OverlayKind::Pheromones));
    }
    if path.contains("/visibilitypanel/fertility/toggle") {
        return Some(UiAction::ToggleOverlay(OverlayKind::Zones));
    }
    if path.contains("/placerbutton/placers/placepellets") {
        return Some(UiAction::Invoke(ControlCommand::SelectTool(
            ToolKind::Pellet,
        )));
    }
    if path.contains("/placerbutton/placers/pherotower") {
        return Some(UiAction::Invoke(ControlCommand::SelectTool(
            ToolKind::PheroTower,
        )));
    }
    None
}

fn control_action(session: &str, node: &Node) -> Option<UiAction> {
    hit_action(node).or_else(|| {
        let label = active_control_label(node);
        functional_action(session, node, &label)
    })
}

fn is_interactable_control(control: &ControlComponent) -> bool {
    matches!(
        control.r#type.as_str(),
        "UnityEngine.UI.Button"
            | "UnityEngine.UI.Selectable"
            | "UnityEngine.UI.Toggle"
            | "UnityEngine.UI.Slider"
            | "UnityEngine.UI.Scrollbar"
            | "UnityEngine.UI.ScrollRect"
    )
}

fn control_kind(node: &Node) -> Option<ControlKind> {
    let types: Vec<&str> = node
        .controls
        .iter()
        .filter(|control| is_interactable_control(control))
        .map(|control| control.r#type.as_str())
        .collect();
    if types
        .iter()
        .any(|kind| *kind == "UnityEngine.UI.Button" || *kind == "UnityEngine.UI.Selectable")
    {
        Some(ControlKind::Button)
    } else if types.contains(&"UnityEngine.UI.Toggle") {
        Some(ControlKind::Toggle)
    } else if types.contains(&"UnityEngine.UI.Slider") {
        Some(ControlKind::Slider)
    } else if types.contains(&"UnityEngine.UI.Scrollbar") {
        Some(ControlKind::Scrollbar)
    } else if types.contains(&"UnityEngine.UI.ScrollRect") {
        Some(ControlKind::ScrollRect)
    } else {
        None
    }
}

fn control_id(session: &str, path: &str) -> ControlId {
    // FNV-1a 64 over session/path; stable across UI sessions and rebuilds.
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in session.bytes().chain(*b"/").chain(path.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Stable id for a Workshop list row (`SelectWorkshopRow`): pass the captured
/// Scroll View path (`.../SubscribedList/body/Scroll View` or Shared list).
#[must_use]
#[allow(dead_code)] // view-dev SelectWorkshopRow wiring
pub fn workshop_row_id(session: &str, scroll_view_path: &str, row: usize) -> ControlId {
    dynamic_row_id(session, scroll_view_path, row)
}

/// Stable id for a dynamic row/widget that is not present in the capture
/// (e.g. save-browser rows): FNV-1a 64 of `session + '/' + path + '#row:' + n`.
#[must_use]
#[allow(dead_code)] // dynamic save-browser rows are bound by view-dev
pub fn dynamic_row_id(session: &str, path: &str, row: usize) -> ControlId {
    control_id(session, &format!("{path}#row:{row}"))
}

fn menu_binding(session: &str, path: &str) -> Option<&'static str> {
    if session != "main_menu_title" {
        return None;
    }
    for (suffix, binding) in [
        ("/Continue", "menu:Continue"),
        ("/NewSim", "menu:New"),
        ("/LoadSim", "menu:Load"),
        ("/Editor", "menu:BibiteLibrary"),
        ("/Settings", "menu:Settings"),
        ("/Achievements", "menu:Achievements"),
        ("/More", "menu:AdditionalTools"),
        ("/Credits", "menu:Credits"),
        ("/Quit", "menu:Quit"),
    ] {
        if path.ends_with(suffix) {
            return Some(binding);
        }
    }
    None
}

fn active_control_label(node: &Node) -> String {
    fn collect(node: &Node, out: &mut Vec<String>) {
        if !node.active_in_hierarchy {
            return;
        }
        for text in &node.texts {
            let value = text.text.trim();
            if !value.is_empty() && !out.iter().any(|existing| existing == value) {
                out.push(value.to_owned());
            }
        }
        for child in &node.children {
            collect(child, out);
        }
    }
    let mut texts = Vec::new();
    collect(node, &mut texts);
    if texts.is_empty() {
        node.name.clone()
    } else {
        texts.join(" / ")
    }
}

fn known_dropdown_options(path: &str, label: &str) -> Option<Vec<String>> {
    let path_lower = path.to_ascii_lowercase();
    let options: Vec<&str> = if path_lower.contains("newgamescenarios") {
        // Exact New Game option lists captured by parity (54d4b50).
        if path_lower.contains("basicdropdown (4)") {
            vec!["Fifty", "One Hundred", "Two Hundred", "Five Hundred", "No"]
        } else if path_lower.contains("basicdropdown (3)") {
            vec!["None", "Lower", "Low", "Normal", "Severe", "Extreme"]
        } else if path_lower.contains("basicdropdown (2)") {
            vec![
                "Sterile",
                "Arid",
                "Low",
                "Normal",
                "High",
                "Lush",
                "Overgrown",
            ]
        } else if path_lower.contains("basicdropdown (1)") {
            vec![
                "Barren", "Sparser", "Sparse", "Low", "Normal", "High", "Dense", "Denser", "Packed",
            ]
        } else if path_lower.contains("basicdropdown") {
            vec![
                "Smallest", "Small", "Normal", "Bigger", "Large", "Larger", "Enormous", "Gigantic",
            ]
        } else {
            return None;
        }
    } else {
        match label {
            "Genetic" => vec!["Genetic", "Maturity", "Both"],
            "Normal (1.0)" => vec![
                "Smaller (0.5)",
                "Small (0.75)",
                "Normal (1.0)",
                "Large (1.15)",
                "Larger (1.25)",
            ],
            "Plant" => vec!["Plant", "Meat"],
            "1080p" => vec!["720p", "768p", "900p", "1080p", "1440p", "4K"],
            "One Minute" => vec![
                "One Minute",
                "Five Minutes",
                "Ten Minutes",
                "Thirty Minutes",
                "One Hour",
                "Five Hours",
                "Ten Hours",
            ],
            _ => return None,
        }
    };
    Some(options.into_iter().map(str::to_owned).collect())
}

fn control_value_meta(node: &Node, label: &str) -> Option<ControlValueMeta> {
    let mut meta = ControlValueMeta::default();
    for control in &node.controls {
        match control.r#type.as_str() {
            "UnityEngine.UI.Toggle" => meta.is_on = control.is_on,
            "UnityEngine.UI.Slider" => {
                meta.value = control.value;
                meta.min_value = control.min_value;
                meta.max_value = control.max_value;
                meta.whole_numbers = control.whole_numbers;
            }
            "UnityEngine.UI.Dropdown" => {
                let mut options = Vec::new();
                fn collect(node: &Node, out: &mut Vec<String>) {
                    if !node.active_in_hierarchy {
                        return;
                    }
                    for text in &node.texts {
                        let value = text.text.trim();
                        if !value.is_empty() && !out.iter().any(|existing| existing == value) {
                            out.push(value.to_owned());
                        }
                    }
                    for child in &node.children {
                        collect(child, out);
                    }
                }
                collect(node, &mut options);
                meta.options = options;
            }
            _ => {}
        }
    }
    #[allow(clippy::collapsible_if)]
    if meta == ControlValueMeta::default()
        && node.name == "Dropdown"
        && let Some(options) = known_dropdown_options(&node.path, label)
    {
        let selected = options.iter().position(|option| option == label);
        meta.options = options;
        meta.selected_index = selected.map(|index| index as u32);
    }
    if meta == ControlValueMeta::default() {
        None
    } else {
        Some(meta)
    }
}

fn collect_session_controls(
    session_name: &str,
    session: &RuntimeSession,
    width: f32,
    height: f32,
    out: &mut Vec<RuntimeControl>,
) {
    let scale = UiFrame::scaler(width, height);
    let reference = canvas_reference(session);
    for root in active_canvas_roots(session) {
        if !is_canvas(root) {
            continue;
        }
        let composed = compose_root(root, reference, RectSemantics::Local);
        if composed.size.x > reference.x * 4.0 || composed.size.y > reference.y * 4.0 {
            // Keep registry and frame composition aligned: world-space/full-world
            // wrappers are skipped by build_session_frame and are not screen UI.
            continue;
        }
        collect_control_node(session_name, root, composed, scale, out);
    }
}

fn collect_control_node(
    session_name: &str,
    node: &Node,
    composed: ComposedRect,
    scale: f32,
    out: &mut Vec<RuntimeControl>,
) {
    if !node.active_in_hierarchy {
        return;
    }
    // Native v2 nodes carry authoritative composed 1920 screen rects. Use the
    // same override in the hit/control registry as the frame composer so visual
    // rects and hit rects cannot drift.
    let composed = if let Some([x0, y0, x1, y1]) = node.native_rect {
        ComposedRect {
            pos: Vec2::new(x0, y0),
            size: Vec2::new((x1 - x0).max(0.0), (y1 - y0).max(0.0)),
            scale: Vec2::new(1.0, 1.0),
        }
    } else {
        composed
    };
    let synthetic_dropdown = node.name == "Dropdown";
    if let Some(kind) =
        control_kind(node).or_else(|| synthetic_dropdown.then_some(ControlKind::Dropdown))
        && !node.path.is_empty()
    {
        let rect = [
            composed.pos.x * scale,
            composed.pos.y * scale,
            (composed.size.x * scale).max(0.0),
            (composed.size.y * scale).max(0.0),
        ];
        let label = active_control_label(node);
        let mut value = control_value_meta(node, &label);
        if kind == ControlKind::Dropdown
            && let Some(meta) = value.as_mut()
        {
            meta.option_rects = meta
                .options
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    [
                        rect[0],
                        rect[1] + rect[3] * (index as f32 + 1.0),
                        rect[2],
                        rect[3].max(18.0),
                    ]
                })
                .collect();
        }
        out.push(RuntimeControl {
            id: control_id(session_name, &node.path),
            session: session_name.to_owned(),
            path: node.path.clone(),
            name: node.name.clone(),
            label,
            kind,
            rect,
            value,
            action: control_action(session_name, node),
            binding: menu_binding(session_name, &node.path),
        });
    }
    let parent_pivot = Vec2::new(node.rect.pivot.x, node.rect.pivot.y);
    let mut children: Vec<&Node> = node.children.iter().collect();
    children.sort_by_key(|child| child.sibling_index);
    for child in children {
        if !child.active_in_hierarchy {
            continue;
        }
        let (size, topdown) = compose_child(
            &child.rect,
            composed.size,
            parent_pivot,
            RectSemantics::Local,
        );
        let child_rect = ComposedRect {
            pos: Vec2::new(
                composed.pos.x + composed.scale.x * topdown.x,
                composed.pos.y + composed.scale.y * topdown.y,
            ),
            size: Vec2::new(size.x * composed.scale.x, size.y * composed.scale.y),
            scale: Vec2::new(
                composed.scale.x * child.rect.local_scale.x,
                composed.scale.y * child.rect.local_scale.y,
            ),
        };
        collect_control_node(session_name, child, child_rect, scale, out);
    }
}

fn log_unmapped_control(control: &RuntimeControl) {
    static LOGGED: OnceLock<Mutex<HashSet<ControlId>>> = OnceLock::new();
    if std::env::var_os("RUSTIBITES_LOG_UNMAPPED_UI").is_none() {
        return;
    }
    let logged = LOGGED.get_or_init(|| Mutex::new(HashSet::new()));
    if let Ok(mut logged) = logged.lock()
        && logged.insert(control.id)
    {
        eprintln!(
            "ui: unmapped control {:016x} session={} path={} label={} rect=({:.0},{:.0} {:.0}x{:.0})",
            control.id,
            control.session,
            control.path,
            control.label,
            control.rect[0],
            control.rect[1],
            control.rect[2],
            control.rect[3],
        );
    }
}

fn hit_action(node: &Node) -> Option<UiAction> {
    if !node.controls.iter().any(is_interactable_control) {
        return None;
    }
    match node.name.as_str() {
        "PauseButton" => Some(UiAction::TogglePause),
        // The play button sets the speed slider to 0 (a log slider: 5^0 = x1) and the target
        // time scale then forces play (`TimeController.SetTarget` / `ForcePlay`).
        "PlayButton" => Some(UiAction::SetSpeedSlider(1.0)),
        "StepButton" | "Step" => Some(UiAction::Step),
        "Button" if node.path.contains("/Popuholder/DialogPopup") => Some(UiAction::DismissPopup),
        "Toggle" if node.path.contains("/VisibilityPanel/Fertility/") => {
            Some(UiAction::ToggleOverlay(OverlayKind::Zones))
        }
        "Toggle" if node.path.contains("/VisibilityPanel/Pheromones/") => {
            Some(UiAction::ToggleOverlay(OverlayKind::Pheromones))
        }
        "RulerToggle" => Some(UiAction::ToggleOverlay(OverlayKind::Ruler)),
        "PlacerButton" => Some(UiAction::ShowToolbar(0)),
        "UserActions" => Some(UiAction::ShowToolbar(1)),
        "ViewButton" => Some(UiAction::ShowToolbar(2)),
        "StatsButton" => Some(UiAction::SetTab(InspectorTab::Stats)),
        // The right-hand "Info" button opens the `InformationPanel` (QuickStatsHolder).
        "OpenQuickStats" => Some(UiAction::Invoke(ControlCommand::OpenInformation)),
        "GeneButton" => Some(UiAction::SetTab(InspectorTab::Genes)),
        "BiologyButton" => Some(UiAction::SetTab(InspectorTab::Biology)),
        "BrainButton" => Some(UiAction::SetTab(InspectorTab::Brain)),
        "CloseButton" => Some(UiAction::ClosePanel),
        // The gear opens the pause menu (`EscapePanel.OpenPanel`); the other buttons of that
        // name belong to the lineage / expanded-brain panels.
        "SettingsButton" if node.path.contains("/UI Buttons/TopRight/") => {
            Some(UiAction::Invoke(ControlCommand::OpenEscapeMenu))
        }
        // The top-right bar-chart button opens the full-screen "Stats and Analysis"
        // (graphs) panel; the "Info +" quick-stats button opens the counts panel.
        "InformationPanelButton" => Some(UiAction::Invoke(ControlCommand::OpenGraphs)),
        "KillBibiteButton" => Some(UiAction::Invoke(ControlCommand::KillSelected)),
        "ReviveCorpseButton" => Some(UiAction::Invoke(ControlCommand::ReviveSelected)),
        "parent1Button" if node.path.contains("/StatsPanel/MainSection/Parents Line/") => {
            Some(UiAction::Invoke(ControlCommand::SelectParent(true)))
        }
        "parent2Button" if node.path.contains("/StatsPanel/MainSection/Parents Line/") => {
            Some(UiAction::Invoke(ControlCommand::SelectParent(false)))
        }
        "LayBibiteEggButton" => Some(UiAction::Invoke(ControlCommand::LayEggs)),
        "BrainHelpButton" => Some(UiAction::Invoke(ControlCommand::OpenBrainHelp)),
        // The Brain panel's Expanded View button: `GUIManager.OpenBibitePanel(ExpendedBrainPanel)`.
        "ExpandedViewerButton" if node.path.contains("/BibitePanels/BrainPanel/") => {
            Some(UiAction::Invoke(ControlCommand::OpenExpandedBrain))
        }
        // `BrainPanelCommon`: Sort is `SortN()`; the toggle is `UserSettings.showBrainDifferences` (shared with the species panel's).
        "SortButton" if node.path.contains("/BibitePanels/BrainPanel/") => {
            Some(UiAction::InspectorBrainSort)
        }
        "BrainDiffToggle" if node.path.contains("/BibitePanels/BrainPanel/") => {
            Some(UiAction::BrainToggleDifferences)
        }
        "SpeciesHolder"
            if node.path.contains("/GenesPanel/") || node.path.contains("/BrainPanel/") =>
        {
            Some(UiAction::Invoke(ControlCommand::OpenLineageOfSelected))
        }
        "Edit" if node.path.contains("/StatsPanel/ButtonHolder/") => {
            Some(UiAction::Invoke(ControlCommand::EditSelected))
        }
        "Label Holder"
            if node
                .path
                .ends_with("/StatsPanel/MainSection/TagHolder/Label Holder") =>
        {
            Some(UiAction::Invoke(ControlCommand::EditSpeciesTag))
        }
        name if name.trim() == "Button" && node.path.contains("/Alive Children/") => {
            Some(UiAction::Invoke(ControlCommand::SelectRandomChild))
        }
        // `BibiteStatsPanel.SaveCurrentBibite` saves the bibite (not the world).
        "Save" if node.path.contains("/StatsPanel/ButtonHolder/") => {
            Some(UiAction::Invoke(ControlCommand::SaveSelected))
        }
        "OpenGenes" => Some(UiAction::SetTab(InspectorTab::Genes)),
        name if name.contains("GridToggle") => Some(UiAction::ToggleOverlay(OverlayKind::Grid)),
        name if name.contains("ZoneToggle") => Some(UiAction::ToggleOverlay(OverlayKind::Zones)),
        name if name.contains("PelletToggle") => {
            Some(UiAction::ToggleOverlay(OverlayKind::Pellets))
        }
        name if name.contains("PheromoneToggle") => {
            Some(UiAction::ToggleOverlay(OverlayKind::Pheromones))
        }
        name if name.contains("HudToggle") => Some(UiAction::ToggleOverlay(OverlayKind::Hud)),
        // Save/tools/lineage/library and settings-value controls stay exposed as
        // ActivateControl ids until their panel bindings land.
        _ => None,
    }
}

fn color_to_rgba(color: Color) -> Rgba {
    [
        (color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.a.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

/// Reads the gzip dump into the plain-JSON cache (best effort) and returns the
/// JSON text. Cache key: source file (size, mtime).
/// Parses one native session file (`<name>.json.gz`, a single session object).
fn load_session_file(path: &Path) -> Result<RuntimeSession, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let mut json = String::new();
    flate2::read::GzDecoder::new(std::io::BufReader::new(file))
        .read_to_string(&mut json)
        .map_err(|error| format!("could not decompress {}: {error}", path.display()))?;
    #[derive(Deserialize)]
    struct StreamedSession {
        #[serde(default)]
        screen: ScreenSize,
        #[serde(default, deserialize_with = "null_vec")]
        canvases: Vec<StreamedNode>,
        #[serde(default, deserialize_with = "null_vec")]
        core_panel_paths: Vec<String>,
        #[serde(default)]
        core_panel_count: usize,
        #[serde(default)]
        node_count: usize,
    }
    let parsed: StreamedSession = serde_json::from_str(&json)
        .map_err(|error| format!("invalid native session {}: {error}", path.display()))?;
    drop(json);
    Ok(RuntimeSession {
        screen: parsed.screen,
        canvases: parsed
            .canvases
            .into_iter()
            .map(|canvas| canvas.finish(true))
            .collect(),
        core_panel_paths: parsed.core_panel_paths,
        core_panel_count: parsed.core_panel_count,
        node_count: parsed.node_count,
    })
}

fn read_cached_json(path: &Path) -> Result<String, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("could not stat {}: {error}", path.display()))?;
    let cache = cache_path(path, &metadata);
    if let Some(cache) = &cache {
        if let Ok(text) = std::fs::read_to_string(cache) {
            return Ok(text);
        }
    }
    let file = std::fs::File::open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let decoder = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    let mut decoder = decoder;
    let mut json = String::new();
    decoder
        .read_to_string(&mut json)
        .map_err(|error| format!("could not decompress {}: {error}", path.display()))?;
    if let Some(cache) = cache {
        if let Some(parent) = cache.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&cache, &json);
    }
    Ok(json)
}

fn workspace_target() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
}

fn cache_path(source: &Path, metadata: &std::fs::Metadata) -> Option<PathBuf> {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs());
    let stem = source.file_stem()?.to_string_lossy();
    Some(
        workspace_target()
            .join("ui_runtime_cache")
            .join(format!("{stem}-{}-{modified}.json", metadata.len())),
    )
}

/// Reads the measured HUD ruler value from the committed save-state capture
/// metadata (`ui_save_capture.json` -> `ruler.value`, e.g. "200u").
fn read_ruler_value(source: &Path) -> Option<f32> {
    // tools/ui/ui_runtime_atlas.json.gz -> repo root
    let root = source.parent()?.parent()?.parent()?;
    let metadata = root.join("assets/game/reference/default_save/ui_save_capture.json");
    let text = std::fs::read_to_string(metadata).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let raw = value.get("ruler")?.get("value")?.as_str()?;
    let number: String = raw
        .chars()
        .filter(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    number.parse().ok()
}

/// Composes a node's screen rect inside one session (reference space).
fn compose_path(session: &RuntimeSession, target: &str) -> Option<ComposedRect> {
    for root in active_canvas_roots(session) {
        if !is_canvas(root) {
            continue;
        }
        let reference = canvas_reference(session);
        let composed = compose_root(root, reference, RectSemantics::Local);
        if let Some(found) = find_composed(root, composed, RectSemantics::Local, target) {
            return Some(found);
        }
    }
    None
}

fn find_composed(
    node: &Node,
    composed: ComposedRect,
    semantics: RectSemantics,
    target: &str,
) -> Option<ComposedRect> {
    if node.path == target {
        return Some(composed);
    }
    let parent_pivot = Vec2::new(node.rect.pivot.x, node.rect.pivot.y);
    let mut children: Vec<&Node> = node.children.iter().collect();
    children.sort_by_key(|child| child.sibling_index);
    for child in children {
        if !child.active_in_hierarchy {
            continue;
        }
        let (size, topdown) = compose_child(&child.rect, composed.size, parent_pivot, semantics);
        let child_rect = ComposedRect {
            pos: Vec2::new(
                composed.pos.x + composed.scale.x * topdown.x,
                composed.pos.y + composed.scale.y * topdown.y,
            ),
            size: Vec2::new(size.x * composed.scale.x, size.y * composed.scale.y),
            scale: Vec2::new(
                composed.scale.x * child.rect.local_scale.x,
                composed.scale.y * child.rect.local_scale.y,
            ),
        };
        if let Some(found) = find_composed(child, child_rect, semantics, target) {
            return Some(found);
        }
    }
    None
}

/// Draws the HUD ruler bracket + measurement text procedurally.
///
/// The game's `RulerHandle` draws the bracket as a script/mesh (no UI Image
/// sprites), so the runtime dump only carries the container/child rects and a
/// stale placeholder text. Reconstruct from those rects plus the measured
/// value recorded in `ui_save_capture.json` and the live camera scale.
fn emit_ruler(context: &mut FrameContext<'_>, runtime: &RuntimeUi, session: &RuntimeSession) {
    let Some(ruler) = compose_path(session, "UICanvas/MainUI/Layer0/Ruler") else {
        return;
    };
    let Some(app) = context.app else {
        return;
    };
    // `UserSettings.ShowRuler` (Visibility panel toggle).
    if !app.show_ruler {
        return;
    }
    // A pinned value (headless reference renders) wins; the live viewer follows
    // the game's stepping rule (`RulerHandle.UpdateSize`).
    let value = runtime.ruler_value.or(Some(app.ruler_target));
    let Some(value) = value.filter(|value| *value > 0.0) else {
        return;
    };
    let zoom = app.camera.zoom;
    if zoom <= 0.0 || !zoom.is_finite() {
        return;
    }
    let reference_zoom = zoom / context.scale.max(1e-6);
    let length = value * reference_zoom;
    if length < 2.0 {
        return;
    }
    let right = ruler.pos.x + ruler.size.x;
    let bottom = ruler.pos.y + ruler.size.y;
    let tick = compose_path(session, "UICanvas/MainUI/Layer0/Ruler/Ruler (1)")
        .map_or(Vec2::new(5.0, 15.0), |rect| rect.size);
    // The game bracket antialiases one extra pixel above the 15 px tick rects.
    let tick_size = Vec2::new(tick.x.max(1.0), (tick.y + 1.0).max(2.0));
    let line_height = ruler.size.y.max(2.0);
    let white = [255, 255, 255, 255];
    let scale = context.scale;
    let mut rect = |x: f32, y: f32, width: f32, height: f32, color: [u8; 4]| {
        context.frame.draws.push(UiDraw::Rect {
            pos: Vec2::new(x * scale, y * scale),
            size: Vec2::new((width * scale).max(1.0), (height * scale).max(1.0)),
            color,
        });
    };
    // Bracket: right-aligned at the Ruler container's right/bottom edge.
    rect(
        right - length,
        bottom - tick_size.y,
        tick_size.x,
        tick_size.y,
        white,
    );
    rect(
        right - tick_size.x,
        bottom - tick_size.y,
        tick_size.x,
        tick_size.y,
        white,
    );
    rect(
        right - length,
        bottom - line_height,
        length,
        line_height,
        white,
    );
    // Measurement text ("200") centred above the bracket, plus the unit label
    // inside the bracket (matching the game HUD). The game text uses an SDF
    // outline; emulate the extra ~1 px coverage with offset copies so the HUD
    // A/B alpha masks line up (plain fontdue glyphs).
    let centre = right - length * 0.5;
    let long_measurement = length > 100.0;
    let text = if long_measurement {
        // The zoomed-in inspector HUD shows the unit in the label ("200u")
        // above the bracket instead of the inside-bracket bars.
        format!("{value:.0}u")
    } else {
        format!("{value:.0}")
    };
    let text_top = if long_measurement {
        // Long measurements (zoomed-in inspector capture) put the label
        // directly above the bracket instead of leaving the fixed gap.
        bottom - tick_size.y - 18.0
    } else {
        bottom - 48.0
    };
    push_ruler_text(context, &text, centre - 1.5, text_top, 24.0, 1.0, white);
    if !long_measurement {
        // Unit label: the game's bracket shows the "u" as two vertical strokes
        // sitting on the horizontal line (not a separate TMP glyph), so draw
        // them as two 5 px bars matching the overlay mask.
        let bar_width = tick_size.x.max(2.0);
        let bar_top = bottom - tick_size.y;
        let bar_height = (bottom - line_height - bar_top).max(1.0);
        for x in [centre - 6.0, centre] {
            context.frame.draws.push(UiDraw::Rect {
                pos: Vec2::new(x * scale, bar_top * scale),
                size: Vec2::new((bar_width * scale).max(1.0), (bar_height * scale).max(1.0)),
                color: white,
            });
        }
    }
    context.emitted += 12;
}

fn push_ruler_text(
    context: &mut FrameContext<'_>,
    text: &str,
    centre: f32,
    top: f32,
    px_ref: f32,
    _outline: f32,
    color: [u8; 4],
) {
    let scale = context.scale;
    let px = px_ref * scale;
    let estimated = text.chars().count() as f32 * px * 0.52;
    let left = centre * scale - estimated * 0.5;
    // The text rasteriser now applies the TMP-like offset outline to every
    // UiDraw::Text, so the ruler must emit only the main text run.
    context.frame.draws.push(UiDraw::Text {
        text: text.to_owned(),
        // Compensate the shared TMP baseline nudge in the rasteriser: the ruler
        // position was A/B-matched before the global text metrics change.
        pos: Vec2::new(left, top * scale - px * 0.20),
        px,
        color,
        role: FontRole::Bold,
        font: None,
    });
}

#[allow(dead_code)] // kept for tooling/tests that read captured nodes
fn find_node_by_path<'a>(session: &'a RuntimeSession, target: &str) -> Option<&'a Node> {
    fn walk<'a>(node: &'a Node, target: &str) -> Option<&'a Node> {
        if node.path == target {
            return Some(node);
        }
        for child in &node.children {
            if let Some(found) = walk(child, target) {
                return Some(found);
            }
        }
        None
    }
    session
        .canvases
        .iter()
        .find_map(|canvas| walk(canvas, target))
}

impl RuntimeUi {
    #[cfg(test)]
    fn reference(&self) -> Vec2 {
        Vec2::new(self.session.screen.width, self.session.screen.height)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(
        anchor_min: (f32, f32),
        anchor_max: (f32, f32),
        anchored: (f32, f32),
        size_delta: (f32, f32),
        pivot: (f32, f32),
    ) -> RectTransform {
        RectTransform {
            anchor_min: Fraction {
                x: anchor_min.0,
                y: anchor_min.1,
            },
            anchor_max: Fraction {
                x: anchor_max.0,
                y: anchor_max.1,
            },
            anchored_position: Point {
                x: anchored.0,
                y: anchored.1,
            },
            size_delta: Point {
                x: size_delta.0,
                y: size_delta.1,
            },
            pivot: Fraction {
                x: pivot.0,
                y: pivot.1,
            },
            ..RectTransform::default()
        }
    }

    #[test]
    fn root_canvas_is_screen_centred() {
        let node = Node {
            rect: rect(
                (0.0, 0.0),
                (0.0, 0.0),
                (0.0, 0.0),
                (1920.0, 1080.0),
                (0.5, 0.5),
            ),
            ..Node::default()
        };
        let composed = compose_root(&node, Vec2::new(1920.0, 1080.0), RectSemantics::Local);
        assert!(
            (composed.pos.x - 0.0).abs() < 0.01,
            "x = {}",
            composed.pos.x
        );
        assert!(
            (composed.pos.y - 0.0).abs() < 0.01,
            "y = {}",
            composed.pos.y
        );
        assert_eq!(composed.size.x, 1920.0);
        assert_eq!(composed.size.y, 1080.0);
    }

    #[test]
    fn stretch_child_fills_parent() {
        let child = rect((0.0, 0.0), (1.0, 1.0), (0.0, 0.0), (0.0, 0.0), (0.5, 0.5));
        let (size, topdown) = compose_child(
            &child,
            Vec2::new(1920.0, 1080.0),
            Vec2::new(0.5, 0.5),
            RectSemantics::Local,
        );
        assert_eq!(size, Vec2::new(1920.0, 1080.0));
        assert!(topdown.x.abs() < 0.01 && topdown.y.abs() < 0.01);

        // Stretched anchors must ignore the pivot for the rect position: a
        // pivot-(0,1) full-stretch panel still covers its parent (this was the
        // Layer1/Right-Panels off-screen bug: it shifted by half the screen).
        let stretched = rect((0.0, 0.0), (1.0, 1.0), (0.0, 0.0), (0.0, 0.0), (0.0, 1.0));
        let (size, topdown) = compose_child(
            &stretched,
            Vec2::new(1920.0, 1080.0),
            Vec2::new(0.5, 0.5),
            RectSemantics::Local,
        );
        assert_eq!(size, Vec2::new(1920.0, 1080.0));
        assert!(topdown.x.abs() < 0.01, "x = {}", topdown.x);
        assert!(topdown.y.abs() < 0.01, "y = {}", topdown.y);
    }

    #[test]
    fn bottom_left_panel_matches_runtime_capture() {
        // UICanvas/MainUI/Layer0/UI Buttons/BottomLeft: anchors 0,0 pivot 0,0
        // size 190x70 inside a 1920x1080 parent.
        let bottom_left = rect(
            (0.0, 0.0),
            (0.0, 0.0),
            (0.0, 0.0),
            (190.0, 70.0),
            (0.0, 0.0),
        );
        let (size, topdown) = compose_child(
            &bottom_left,
            Vec2::new(1920.0, 1080.0),
            Vec2::new(0.5, 0.5),
            RectSemantics::Local,
        );
        assert_eq!(size, Vec2::new(190.0, 70.0));
        assert!((topdown.x - 0.0).abs() < 0.01);
        assert!((topdown.y - 1010.0).abs() < 0.01, "y = {}", topdown.y);

        // PlacerButton: anchors (0,1), anchored (35,-35), pivot .5, size 50.
        let placer = rect(
            (0.0, 1.0),
            (0.0, 1.0),
            (35.0, -35.0),
            (50.0, 50.0),
            (0.5, 0.5),
        );
        let (size, local) = compose_child(&placer, size, Vec2::new(0.0, 0.0), RectSemantics::Local);
        assert_eq!(size, Vec2::new(50.0, 50.0));
        assert!((local.x - 10.0).abs() < 0.01, "x = {}", local.x);
        assert!((local.y - 10.0).abs() < 0.01, "y = {}", local.y);
    }

    #[test]
    fn loader_rejects_missing_file() {
        let result = RuntimeUi::load(Path::new("definitely/not/a/runtime-ui-dump.json.gz"));
        assert!(result.is_err());
    }

    #[test]
    fn parses_minimal_session() {
        let json = r#"{
            "schema": "rustibites.assets.ui_runtime_atlas.v1",
            "sessions": {
                "default_world": {
                    "screen": {"width": 1920, "height": 1080},
                    "node_count": 2,
                    "core_panel_count": 1,
                    "core_panel_paths": ["UICanvas/MainUI/Layer0/UI Buttons/BottomLeft"],
                    "canvases": [
                        {
                            "name": "UICanvas", "path": "UICanvas",
                            "active": true, "active_in_hierarchy": true,
                            "rect": {"anchor_min": {"x":0,"y":0}, "anchor_max": {"x":0,"y":0},
                                     "anchored_position": {"x":0,"y":0}, "size_delta": {"x":1920,"y":1080},
                                     "pivot": {"x":0.5,"y":0.5}, "local_position": {"x":0,"y":0,"z":0},
                                     "local_rotation": {"x":0,"y":0,"z":0,"w":1},
                                     "local_scale": {"x":1,"y":1,"z":1}},
                            "controls": [{"type": "UnityEngine.Canvas", "enabled": true}],
                            "children": [
                                {"name": "Badge", "path": "UICanvas/Badge",
                                 "active": true, "active_in_hierarchy": true,
                                 "rect": {"anchor_min": {"x":0,"y":1}, "anchor_max": {"x":0,"y":1},
                                          "anchored_position": {"x":20,"y":-20}, "size_delta": {"x":40,"y":20},
                                          "pivot": {"x":0.5,"y":0.5}, "local_position": {"x":20,"y":-20,"z":0},
                                          "local_rotation": {"x":0,"y":0,"z":0,"w":1},
                                          "local_scale": {"x":1,"y":1,"z":1}},
                                 "texts": [{"type":"TMPro.TextMeshProUGUI","text":"hello",
                                            "font":{"name":"Roboto-Regular SDF","missing":false},
                                            "font_size":14,"alignment":"Left",
                                            "color":{"r":1,"g":1,"b":1,"a":1}}],
                                 "children": []}
                            ]
                        }
                    ]
                }
            }
        }"#;
        let runtime = RuntimeUi::from_json(json).expect("parse");
        assert_eq!(runtime.session.canvases.len(), 1);
        assert_eq!(runtime.session.node_count, 2);
        assert_eq!(runtime.session.core_panel_paths.len(), 1);
        let stats = runtime.stats();
        assert_eq!(stats.nodes, 2);
        assert_eq!(stats.active_nodes, 2);
        assert_eq!(stats.active_texts, 1);
        let panels = runtime.core_panels();
        assert!(panels.iter().any(|panel| panel.paths == 1));
    }

    #[test]
    fn committed_dump_composes_active_hud_when_present() {
        let Ok(Some(runtime)) = RuntimeUi::discover() else {
            eprintln!("skipping: committed runtime UI dump absent");
            return;
        };
        assert!(!runtime.session.canvases.is_empty());
        assert!(runtime.session.core_panel_paths.len() >= 100);
        let stats = runtime.stats();
        assert!(stats.nodes > 1000, "nodes = {}", stats.nodes);
        assert!(stats.active_nodes > 0);
        // Walk the active tree and check the known bottom-left button position
        // (10, 1020) in the 1920x1080 reference space: regression for the
        // anchor/pivot composer.
        let compositions = composed_active_nodes(&runtime);
        assert!(!compositions.is_empty());
        let placer = compositions
            .iter()
            .find(|(path, _)| path.ends_with("/BottomLeft/PlacerButton"))
            .map(|(_, rect)| *rect)
            .expect("active PlacerButton in committed dump");
        assert!(
            (placer.pos.x - 10.0).abs() < 0.5,
            "placer x = {}",
            placer.pos.x
        );
        assert!(
            (placer.pos.y - 1020.0).abs() < 0.5,
            "placer y = {}",
            placer.pos.y
        );
        // Time text is top-right; sanity-check it is inside the reference screen.
        assert!(placer.pos.x >= 0.0 && placer.pos.y >= 0.0);
    }

    fn composed_active_nodes(runtime: &RuntimeUi) -> Vec<(String, ComposedRect)> {
        let mut out = Vec::new();
        for root in active_canvas_roots(&runtime.session) {
            let composed = compose_root(root, runtime.reference(), RectSemantics::Local);
            collect_composed(root, composed, RectSemantics::Local, &mut out);
        }
        out
    }

    fn collect_composed(
        node: &Node,
        composed: ComposedRect,
        semantics: RectSemantics,
        out: &mut Vec<(String, ComposedRect)>,
    ) {
        out.push((node.path.clone(), composed));
        let parent_pivot = Vec2::new(node.rect.pivot.x, node.rect.pivot.y);
        for child in &node.children {
            let (size, topdown) =
                compose_child(&child.rect, composed.size, parent_pivot, semantics);
            let child_rect = ComposedRect {
                pos: Vec2::new(
                    composed.pos.x + composed.scale.x * topdown.x,
                    composed.pos.y + composed.scale.y * topdown.y,
                ),
                size: Vec2::new(size.x * composed.scale.x, size.y * composed.scale.y),
                scale: Vec2::new(
                    composed.scale.x * child.rect.local_scale.x,
                    composed.scale.y * child.rect.local_scale.y,
                ),
            };
            if child.active_in_hierarchy {
                collect_composed(child, child_rect, semantics, out);
            }
        }
    }

    fn workspace_path(relative: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join(relative)
    }

    fn fixture(relative: &str) -> Option<(RuntimeUi, App, UiAssets)> {
        let path = workspace_path(relative);
        if !path.is_file() {
            eprintln!("skipping: {relative} absent");
            return None;
        }
        let Ok(Some(runtime)) = RuntimeUi::discover() else {
            eprintln!("skipping: runtime UI dump absent");
            return None;
        };
        let Ok(Some(atlas)) = UiAssets::discover() else {
            eprintln!("skipping: UI atlas absent");
            return None;
        };
        let Ok(loaded) = crate::app::LoadedWorld::load(Some(&path), 12345, 0, 0) else {
            eprintln!("skipping: {relative} could not be loaded");
            return None;
        };
        let mut app = App::new(loaded, Vec2::new(1280.0, 800.0), None);
        app.refresh_overlay(0.0);
        Some((runtime, app, atlas))
    }

    /// The viewer's own runtime (the native capture, whose list headers and extras have
    /// geometry; `discover()` gives the old dump) with a world and the UI atlas.
    #[test]
    fn a_label_in_the_bold_style_is_drawn_with_tmps_synthetic_bold_whatever_its_font() {
        let Some((runtime, _, atlas)) = fixture("testdata/parity/default/rust_seed_600.zip") else {
            return;
        };
        let frame = runtime
            .build_menu_frame(false, &atlas, 1920.0, 1080.0, None)
            .expect("the title frame");
        // `VersionText` is `Boston Black SDF` 28 in the Bold style: the game draws it 0.07 em wider per character and heavier.
        let version = frame
            .draws
            .iter()
            .find_map(|draw| match draw {
                UiDraw::Text { text, font, .. } if text.starts_with("The Bibites 0.") => {
                    Some(font.clone())
                }
                _ => None,
            })
            .expect("the version label");
        assert_eq!(version.as_deref(), Some("Boston Black SDF +bold"));
    }

    fn native_fixture() -> Option<(RuntimeUi, App, UiAssets)> {
        let native = workspace_path("assets/game/ui_runtime_native.json.gz");
        let save = workspace_path("testdata/parity/default/rust_seed_600.zip");
        if !native.is_file() || !save.is_file() {
            eprintln!("skipping: the native runtime dump or the test save is absent");
            return None;
        }
        let Ok(runtime) = RuntimeUi::load_native(&native) else {
            eprintln!("skipping: the native runtime dump could not be loaded");
            return None;
        };
        let Ok(Some(atlas)) = UiAssets::discover() else {
            eprintln!("skipping: UI atlas absent");
            return None;
        };
        let Ok(loaded) = crate::app::LoadedWorld::load(Some(&save), 12345, 0, 0) else {
            eprintln!("skipping: the test save could not be loaded");
            return None;
        };
        let mut app = App::new(loaded, Vec2::new(1280.0, 800.0), None);
        app.refresh_overlay(0.0);
        Some((runtime, app, atlas))
    }

    fn default_fixture() -> Option<(RuntimeUi, App, UiAssets)> {
        fixture("testdata/real/Default.zip")
    }

    fn save_fixture() -> Option<(RuntimeUi, App, UiAssets)> {
        fixture("testdata/parity/default/rust_seed_600.zip")
    }

    /// The window builds the UI of a frame in which the app only watches (`--sim-thread`) with a husk of a world in place of
    /// the real one: whatever the default frame reads of the world, the husk must give the same answer as the world did.
    /// This fails when a part of the default UI starts reading the world's content (as the pellet tooltip did) without the
    /// thread mode being told.
    #[test]
    fn the_runtime_ui_of_an_app_that_only_watches_does_not_read_the_world() {
        let Some((runtime, mut app, atlas)) = default_fixture() else {
            return;
        };
        app.prefs.min_fps = 0;
        app.enable_sim_thread();
        let started = std::time::Instant::now();
        while !app.sim.as_mut().is_some_and(|sim| sim.is_ready()) {
            assert!(started.elapsed().as_secs() < 20, "the thread did not start");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // Fast enough for the world's clock to read something other than the husk's zero by the time the world leaves.
        app.set_target_speed(60.0);
        let idle = crate::app::FrameInput::default();
        for _ in 1..crate::app::LIGHT_FRAMES_BEFORE_AWAY {
            app.frame_world(&idle);
            app.advance(1.0 / 60.0);
        }
        assert!(!app.world_away());
        assert!(
            app.world.simulated_time() >= 5.0,
            "the world ran {} s",
            app.world.simulated_time()
        );
        let home = runtime.build_frame(&app, &atlas, 1280.0, 800.0);
        // The next frame sends the world away; nothing is simulated before the frame is built.
        app.frame_world(&idle);
        assert!(app.world_away());
        assert_eq!(app.world.bibites.len(), 0, "the husk is empty");
        let away = runtime.build_frame(&app, &atlas, 1280.0, 800.0);
        assert_eq!(home.draws.len(), away.draws.len());
        for (index, (a, b)) in home.draws.iter().zip(&away.draws).enumerate() {
            assert_eq!(format!("{a:?}"), format!("{b:?}"), "draw {index}");
        }
        assert_eq!(home.hits, away.hits);
        assert_eq!(home.actions, away.actions);
        assert_eq!(format!("{:?}", home.tips), format!("{:?}", away.tips));
    }

    #[test]
    fn a_slider_rows_edit_field_takes_the_place_of_its_slider_and_reset_button() {
        use crate::dyn_panel::FieldRef;
        let Some((runtime, mut app, atlas)) = native_fixture() else {
            return;
        };
        app.panels.open(PanelKind::WorldDynamics);
        // The Zones tab starts with a group of sliders (the Simulation tab's are below the fold).
        app.dyn_panel.tab = 1;
        let acts = |frame: &UiFrame| -> Vec<UiAction> {
            frame.actions.iter().map(|(_, action)| *action).collect()
        };
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        let before = acts(&frame);
        // Every slider row's value text is its edit button.
        let (index, field) = before
            .iter()
            .find_map(|action| match action {
                UiAction::DynEditValue(field @ FieldRef::Setting(index)) => Some((*index, *field)),
                _ => None,
            })
            .expect("a slider row");
        assert!(
            before
                .iter()
                .any(|action| matches!(action, UiAction::DynSlide(at, ..) if *at == index))
        );
        assert!(before.contains(&UiAction::DynReset(index)));
        assert!(!before.contains(&UiAction::ValueEditInside));
        // Opened, the slider and the reset button leave the row and the field takes their place.
        app.apply_ui_actions(&[UiAction::DynEditValue(field)]);
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        let open = acts(&frame);
        assert!(
            !open
                .iter()
                .any(|action| matches!(action, UiAction::DynSlide(at, ..) if *at == index))
        );
        assert!(!open.contains(&UiAction::DynReset(index)));
        assert!(open.contains(&UiAction::ValueEditInside));
        assert!(
            open.contains(&UiAction::DynEditValue(field)),
            "the value text stays the row's edit button"
        );
        // The other rows are as they were.
        assert!(
            open.iter()
                .filter(|a| matches!(a, UiAction::DynSlide(..)))
                .count()
                + 1
                == before
                    .iter()
                    .filter(|a| matches!(a, UiAction::DynSlide(..)))
                    .count()
        );
        // The field is where the row's slider was, at least 300 wide and 34 high, and the
        // selected text is drawn on its highlight.
        let (row_index, row) = crate::dyn_panel::ui_of(match field {
            FieldRef::Setting(at) => crate::setting_ui_data::SETTING_UI[usize::from(at)].field,
            _ => unreachable!(),
        })
        .unwrap();
        assert_eq!(row_index, usize::from(index));
        let edit = app.value_edit.as_ref().expect("open").clone();
        let texts = |frame: &UiFrame| -> Vec<String> {
            frame
                .draws
                .iter()
                .filter_map(|draw| match draw {
                    UiDraw::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect()
        };
        assert!(texts(&frame).contains(&edit.text), "{:?}", texts(&frame));
        // Typing shows in the field and in the value text (`UpdateValueDuringEdit`).
        app.typed = "7".to_owned();
        app.handle_input(&crate::app::FrameInput {
            dt: 0.016,
            ..crate::app::FrameInput::default()
        });
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        let drawn = texts(&frame);
        assert!(drawn.contains(&"7".to_owned()), "{drawn:?}");
        let (mode, _) = crate::dyn_panel::slider_info_or_default(row);
        let rules = crate::dyn_panel::setting_rules(row, mode);
        let preview = crate::dyn_panel::format_of(row).format(rules.preview("7").unwrap());
        assert!(drawn.contains(&preview), "{preview:?} in {drawn:?}");
        // With the text deleted the field shows its placeholder.
        app.handle_input(&crate::app::FrameInput {
            dt: 0.016,
            backspace: true,
            ..crate::app::FrameInput::default()
        });
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert!(texts(&frame).contains(&"Enter value...".to_owned()));
    }

    #[test]
    fn the_zone_confirmations_use_the_games_wording() {
        use crate::app::ConfirmKind;
        let (title, text, buttons) = confirm_texts(&ConfirmKind::DeleteZone(3), "Zone 3");
        assert_eq!(title, "Delete Zone");
        assert_eq!(
            text,
            "Are you sure you want to delete Zone 3?\nYou will lose all data"
        );
        assert_eq!(buttons, ["Cancel", "YES"]);
        let (title, text, buttons) = confirm_texts(&ConfirmKind::ClearZones, "");
        assert_eq!(title, "Delete All Zones");
        assert_eq!(
            text,
            "Are you sure you want to delete all zones? You will lose all data"
        );
        assert_eq!(buttons, ["Cancel", "YES"]);
    }

    #[test]
    fn the_worlds_delete_save_confirmation_is_the_warning_popup() {
        use crate::app::ConfirmKind;
        let Some((runtime, mut app, atlas)) = native_fixture() else {
            return;
        };
        app.confirm_dialog = Some(ConfirmKind::DeleteSave(std::path::PathBuf::new()));
        let scale = UiFrame::scaler(1920.0, 1080.0);
        let mut frame = UiFrame::new(scale);
        runtime.emit_confirm_dialog(&mut frame, &app, &atlas, None, 1920.0, 1080.0);
        let has = |wanted: UiAction| frame.actions.iter().any(|(_, action)| *action == wanted);
        // Cancel, the proceed button and the "do not show again" checkbox, over a blocker.
        assert!(has(UiAction::Invoke(ControlCommand::Cancel)));
        assert!(has(UiAction::Invoke(ControlCommand::Confirm)));
        assert!(has(UiAction::Invoke(ControlCommand::ToggleDoNotShowAgain)));
        assert!(
            has(UiAction::Consume),
            "the screen blocker takes the clicks"
        );
        // The popup is the 400-wide prefab scaled by the world's holder (1.35): 540 px.
        let panel = frame
            .hits
            .iter()
            .filter(|hit| hit.2.x < 1920.0 && hit.2.x > 400.0)
            .map(|hit| hit.2.x)
            .next()
            .expect("the panel's hit area");
        assert!((panel - 540.0 * scale).abs() < 0.5, "{panel}");
        // The title and the text are the game's.
        let words: String = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("|");
        assert!(words.contains("Deleting Save File"), "{words}");
        // The text wraps at 380 units (without the loaded fonts a character is half an em).
        let flat = words.replace('|', " ");
        assert!(
            flat.contains("Are you sure you want to delete this save file?"),
            "{words}"
        );
        assert!(words.contains("do not show again"), "{words}");
        assert!(words.contains("Cancel") && words.contains("Yes"), "{words}");
        // A choice dialog has no checkbox.
        app.confirm_dialog = Some(ConfirmKind::KillHalf);
        let mut frame = UiFrame::new(scale);
        runtime.emit_confirm_dialog(&mut frame, &app, &atlas, None, 1920.0, 1080.0);
        assert!(
            !frame.actions.iter().any(
                |(_, action)| *action == UiAction::Invoke(ControlCommand::ToggleDoNotShowAgain)
            )
        );
    }

    #[test]
    fn the_load_games_list_is_made_of_save_item_rows_that_scroll_by_the_prefabs_step() {
        use crate::ui::MenuCommand;
        let Some((runtime, _, atlas)) = window_fixture() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("rustibites-loadrows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        for index in 0..30 {
            std::fs::write(dir.join(format!("save{index:02}.zip")), b"x").expect("save");
        }
        let mut menu =
            crate::menu::MenuState::new(None, &dir).with_private_settings(&dir.join("s.json"));
        menu.move_selection(2);
        let _ = menu.activate();
        assert_eq!(menu.view(), crate::menu::MenuView::LoadGame);
        let frame = runtime
            .build_menu_view_frame(&menu, &atlas, 1920.0, 1080.0, None)
            .expect("the load game frame");
        // `SaveItem` rows: 805 x 50 at x 90, 55 apart from y 140 (under the "Include autosaves" line).
        let rect_of = |frame: &UiFrame, row: u16| {
            let (id, _) = frame
                .actions
                .iter()
                .find(|(_, action)| *action == UiAction::Menu(MenuCommand::SelectSave(row)))?;
            frame
                .hits
                .iter()
                .find(|hit| hit.0 == *id)
                .map(|(_, pos, size)| (*pos, *size))
        };
        // 30 rows do not fit: the scrollbar is there, and the viewport gives up its 17 (788 wide rows).
        for row in 0..3_u16 {
            let (pos, size) = rect_of(&frame, row).expect("a row");
            assert_eq!((pos.x, pos.y), (90.0, 140.0 + 55.0 * f32::from(row)));
            assert_eq!((size.x, size.y), (788.0, 50.0));
        }
        let count = |frame: &UiFrame, wanted: MenuCommand| {
            frame
                .actions
                .iter()
                .filter(|(_, action)| *action == UiAction::Menu(wanted))
                .count()
        };
        // At the top: the handle and the page under it (the track above it has no room).
        assert_eq!(count(&frame, MenuCommand::ListBarHandle), 1);
        assert_eq!(count(&frame, MenuCommand::ListBarPage(1)), 1);
        assert_eq!(count(&frame, MenuCommand::ListBarPage(-1)), 0);
        // Only the rows that fit entirely are there: 865 high from 140 holds 15 of them.
        assert!(rect_of(&frame, 14).is_some() && rect_of(&frame, 15).is_none());
        // A wheel notch moves a list by the prefab's `ScrollRect.scrollSensitivity` (100), as far as the content goes (30 rows:
        // 15 + 50 + 30 * 55 - 5 + 50 = 1760 in a view 930 high).
        assert_eq!(menu.wheel_step(), 100.0);
        for _ in 0..30 {
            menu.scroll_list(menu.wheel_step());
        }
        menu.settle_scroll();
        assert_eq!(menu.list_scroll(), 830.0);
        let frame = runtime
            .build_menu_view_frame(&menu, &atlas, 1920.0, 1080.0, None)
            .expect("the load game frame");
        assert!(rect_of(&frame, 0).is_none(), "the first rows scrolled away");
        assert!(rect_of(&frame, 29).is_some(), "and the last one is in view");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_credits_tab_scrolls_under_its_mask_and_its_buttons_are_urlmanagers() {
        use crate::menu::MenuView;
        use crate::ui::MenuCommand;
        let Some((runtime, _, atlas)) = window_fixture() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("rustibites-credits-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut menu =
            crate::menu::MenuState::new(None, &dir).with_private_settings(&dir.join("s.json"));
        menu.open_view_for_capture(MenuView::Credits);
        let frame_of = |menu: &crate::menu::MenuState| {
            runtime
                .build_menu_view_frame(menu, &atlas, 1920.0, 1080.0, None)
                .expect("the credits frame")
        };
        // As the game opens it: the end of the credits (the prefab stores the content scrolled), what the mask hides above is not drawn, and the links
        // are not in view.
        let frame = frame_of(&menu);
        let texts = drawn_texts(&frame);
        assert!(texts.iter().any(|text| text == "Special Thanks to"));
        assert!(texts.iter().any(|text| text == "Yuronokage"));
        assert!(
            !texts
                .iter()
                .any(|text| text == "Development Team" || text.contains("Caussan")),
            "{texts:?}"
        );
        for which in [2, 3, 4, 5, 6] {
            assert!(hit_of(&frame, UiAction::Menu(MenuCommand::OpenUrl(which))).is_none());
        }
        // The bar is live: its handle is where the capture had it, and the page above the handle is a hit.
        let handle = hit_of(&frame, UiAction::Menu(MenuCommand::ListBarHandle)).expect("handle");
        assert!((handle.0.y - 467.1).abs() < 0.1 && (handle.1.y - 582.8).abs() < 0.1);
        assert!(hit_of(&frame, UiAction::Menu(MenuCommand::ListBarPage(-1))).is_some());
        // To the top (a notch is 35): the team, the Patreon sentence (its "<3" is TMP's text, not a tag) and the five buttons.
        for _ in 0..20 {
            menu.scroll_list(-menu.wheel_step());
        }
        menu.settle_scroll();
        assert_eq!(menu.list_scroll(), 0.0);
        let frame = frame_of(&menu);
        let texts = drawn_texts(&frame);
        assert!(texts.iter().any(|text| text == "Development Team"));
        assert!(texts.iter().any(|text| text.contains("Caussan")));
        assert!(!texts.iter().any(|text| text == "Special Thanks to"));
        assert!(texts.iter().any(|text| text.ends_with("<3")), "{texts:?}");
        // Left to right on the screen they are Patreon, YouTube, Twitter, BlueSky and Reddit (the order of the layout, not of the buttons' names),
        // 60 x 60, 125 apart, from x 1107.7, 140 down (the capture's 623 less).
        let spot = |which: u8| {
            let (pos, size) = hit_of(&frame, UiAction::Menu(MenuCommand::OpenUrl(which)))
                .expect("a social button");
            (pos.x, pos.y, size.x, size.y)
        };
        for (slot, which) in [2_u8, 6, 4, 5, 3].into_iter().enumerate() {
            let (x, y, w, h) = spot(which);
            assert!(
                (x - (1107.7 + 125.0 * slot as f32)).abs() < 0.1,
                "{which}: {x}"
            );
            assert!((y - 139.96).abs() < 0.1, "{which}: {y}");
            assert!(
                (w - 60.0).abs() < 0.01 && (h - 60.0).abs() < 0.01,
                "{which}: {w} x {h}"
            );
        }
        // The Patreon simulation is another scene of the game.
        let (pos, size) = hit_of(&frame, UiAction::Menu(MenuCommand::Unavailable(7)))
            .expect("the Patreon simulation button");
        assert!((pos.x - 1122.0).abs() < 0.1 && (pos.y - 342.66).abs() < 0.1);
        assert!((size.x - 60.0).abs() < 0.01 && (size.y - 60.0).abs() < 0.01);
        // A button under the mask's top answers nothing: a notch down moves the row up 35 (to 104.96, still in view), another one to 69.96 (out).
        menu.scroll_list(menu.wheel_step());
        let frame = frame_of(&menu);
        assert!(hit_of(&frame, UiAction::Menu(MenuCommand::OpenUrl(2))).is_some());
        menu.scroll_list(menu.wheel_step());
        let frame = frame_of(&menu);
        for which in [2, 3, 4, 5, 6] {
            assert!(
                hit_of(&frame, UiAction::Menu(MenuCommand::OpenUrl(which))).is_none(),
                "{which}: cut by the mask"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_tools_tab_wraps_its_paragraphs_and_its_links_answer_on_the_icon_and_the_title() {
        use crate::menu::MenuView;
        use crate::ui::MenuCommand;
        let Some((runtime, _, atlas)) = window_fixture() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("rustibites-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut menu =
            crate::menu::MenuState::new(None, &dir).with_private_settings(&dir.join("s.json"));
        menu.open_view_for_capture(MenuView::Tools);
        let frame_of = |menu: &crate::menu::MenuState| {
            runtime
                .build_menu_view_frame(menu, &atlas, 1920.0, 1080.0, None)
                .expect("the tools frame")
        };
        let frame = frame_of(&menu);
        let texts = drawn_texts(&frame);
        // The intro is a paragraph (`TMP wordWrap`): its lines, put together, are the sentence, and none of them is the sentence.
        let intro = "There are many additional tools available that allow you to expand your understanding of The Bibites or add a few functionalities that are not yet present in-game. Here, I'll try to link you to a few of those.";
        assert!(!texts.iter().any(|text| text == intro));
        let lines: Vec<&str> = texts
            .iter()
            .map(String::as_str)
            .filter(|text| text.len() >= 4 && intro.contains(text))
            .collect();
        assert!(lines.len() >= 2, "{lines:?}");
        assert_eq!(lines.join(" "), intro);
        // The section titles are single lines (their boxes are as wide as they are).
        for title in [
            "The Bibites' Wikipedia",
            "The Bibite Research Conglomerate",
            "The Bibites Patreon Simulation",
            "The Custom Sprite Editor",
        ] {
            assert!(texts.iter().any(|text| text == title), "{title}");
        }
        // The last description is cut by the mask line by line: its first line is there, its last one is not.
        assert!(
            texts
                .iter()
                .any(|text| text.starts_with("As the bibites have procedural")),
            "{texts:?}"
        );
        assert!(!texts.iter().any(|text| text.contains("available to users")));
        // A link answers on its icon and on its title (what is a graphic under its `Button`): the wiki and the research Discord open
        // `URLManager`'s pages, the Patreon simulation and the sprite editor are other scenes of the game.
        let count = |wanted: MenuCommand| {
            frame
                .actions
                .iter()
                .filter(|(_, action)| *action == UiAction::Menu(wanted))
                .count()
        };
        assert_eq!(count(MenuCommand::OpenUrl(0)), 2);
        assert_eq!(count(MenuCommand::OpenUrl(1)), 2);
        assert_eq!(count(MenuCommand::Unavailable(7)), 2);
        assert_eq!(count(MenuCommand::Unavailable(5)), 2);
        let (pos, size) =
            hit_of(&frame, UiAction::Menu(MenuCommand::OpenUrl(0))).expect("the wiki link");
        assert!((pos.x - 954.1).abs() < 0.1 && (pos.y - 271.4).abs() < 0.1);
        assert!((size.x - 60.0).abs() < 0.01 && (size.y - 60.0).abs() < 0.01);
        // To the end (73.32 down): the rows moved up, and the intro's first line went under the mask.
        menu.menu_command(MenuCommand::ListBarPage(1));
        let frame = frame_of(&menu);
        let texts = drawn_texts(&frame);
        assert!(
            !texts
                .iter()
                .any(|text| text.starts_with("There are many additional tools")),
            "{texts:?}"
        );
        assert!(texts.iter().any(|text| text.contains("available to users")));
        let (pos, _) =
            hit_of(&frame, UiAction::Menu(MenuCommand::OpenUrl(0))).expect("the wiki link");
        assert!((pos.y - (271.4 - 73.32)).abs() < 0.1, "{}", pos.y);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rich_text_strips_the_tags_and_keeps_what_is_not_one() {
        assert_eq!(
            strip_rich_text("<b>bold</b> and <color=#ff0000>red</color>"),
            "bold and red"
        );
        // TMP prints what it does not know as a tag: a heart, a comparison, a lone bracket.
        assert_eq!(strip_rich_text("for all this <3"), "for all this <3");
        assert_eq!(strip_rich_text("a < b > c"), "a < b > c");
        assert_eq!(strip_rich_text("end <"), "end <");
        assert_eq!(strip_rich_text("<sprite=1>x"), "x");
    }

    #[test]
    fn the_title_menus_delete_confirmation_is_the_warning_popup() {
        use crate::ui::MenuCommand;
        let Some((runtime, _, atlas)) = native_fixture() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("rustibites-menuwarn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        for file in ["a.zip", "b.zip"] {
            std::fs::write(dir.join(file), b"x").expect("save");
        }
        let mut menu =
            crate::menu::MenuState::new(None, &dir).with_private_settings(&dir.join("s.json"));
        menu.move_selection(2);
        let _ = menu.activate();
        assert_eq!(menu.view(), crate::menu::MenuView::LoadGame);
        menu.menu_command(MenuCommand::SelectSave(0));
        menu.menu_command(MenuCommand::DeleteSave);
        assert!(menu.confirm_delete());
        let frame = runtime
            .build_menu_view_frame(&menu, &atlas, 1920.0, 1080.0, None)
            .expect("the load game frame");
        let count = |wanted: UiAction| {
            frame
                .actions
                .iter()
                .filter(|(_, action)| *action == wanted)
                .count()
        };
        assert_eq!(count(UiAction::Menu(MenuCommand::ConfirmDelete(true))), 1);
        assert_eq!(count(UiAction::Menu(MenuCommand::ConfirmDelete(false))), 1);
        assert_eq!(count(UiAction::Menu(MenuCommand::ToggleDoNotShowAgain)), 1);
        // The title menu's holder is not scaled: a 400-wide popup.
        let panel = frame
            .hits
            .iter()
            .filter(|hit| (hit.2.x - 400.0).abs() < 0.5)
            .count();
        assert!(panel >= 1, "a 400 px wide panel hit");
        // Without the popup, none of it is there.
        menu.menu_command(MenuCommand::ConfirmDelete(false));
        let frame = runtime
            .build_menu_view_frame(&menu, &atlas, 1920.0, 1080.0, None)
            .expect("the load game frame");
        assert!(
            !frame
                .actions
                .iter()
                .any(|(_, action)| *action == UiAction::Menu(MenuCommand::ToggleDoNotShowAgain))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_library_and_settings_screens_fill_the_screen_with_their_opaque_navy_over_a_world_too() {
        use crate::menu::MenuView;
        let Some((runtime, _, atlas)) = native_fixture() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("rustibites-navy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        for (row, view, navy) in [
            (3, MenuView::Library, [0, 0, 27, 255]),
            (4, MenuView::Settings, [0, 0, 31, 255]),
        ] {
            // The title scene, and the world with the screen over it: the same opaque root, nothing behind it shows.
            for overlay in [false, true] {
                let mut menu = crate::menu::MenuState::new(None, &dir)
                    .with_private_settings(&dir.join("s.json"));
                menu.set_overlay(overlay);
                menu.move_selection(row);
                let _ = menu.activate();
                assert_eq!(menu.view(), view);
                let frame = runtime
                    .build_menu_view_frame(&menu, &atlas, 1920.0, 1080.0, None)
                    .expect("the screen's frame");
                assert!(
                    matches!(
                        frame.draws.first(),
                        Some(UiDraw::Rect { pos, size, color })
                            if *pos == Vec2::ZERO && *size == Vec2::new(1920.0, 1080.0) && *color == navy
                    ),
                    "{view:?} (overlay {overlay}): {:?}",
                    frame.draws.first()
                );
                // The title video is not drawn behind them either.
                menu.set_video_background(true);
                assert!(!menu.shows_video(), "{view:?}");
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_challenge_list_shows_the_champion_section_instead_of_the_scenario_modifiers() {
        use crate::menu::MenuView;
        use crate::ui::MenuCommand;
        let Some((runtime, _, atlas)) = native_fixture() else {
            return;
        };
        let (mut menu, _scenario, champion) = crate::menu::MenuState::for_challenge_tests("frame");
        let actions = |menu: &crate::menu::MenuState| -> Vec<UiAction> {
            runtime
                .build_menu_view_frame(menu, &atlas, 1920.0, 1080.0, None)
                .expect("the New Game frame")
                .actions
                .iter()
                .map(|(_, action)| *action)
                .collect()
        };
        // The scenario list: the modifiers' "Start Simulation", no champion section.
        assert_eq!(menu.view(), MenuView::NewGame);
        let scenarios = actions(&menu);
        assert!(scenarios.contains(&UiAction::Menu(MenuCommand::StartSimulation)));
        assert!(!scenarios.contains(&UiAction::Menu(MenuCommand::SelectChampion)));
        // The challenge list, nothing picked: "Select Champion", the scenario launcher, a start that is not a button; no modifiers.
        menu.menu_command(MenuCommand::ToggleChallenges);
        let empty = actions(&menu);
        assert!(!empty.contains(&UiAction::Menu(MenuCommand::StartSimulation)));
        assert!(empty.contains(&UiAction::Menu(MenuCommand::SelectChampion)));
        assert!(empty.contains(&UiAction::Menu(MenuCommand::StartChallenge(false))));
        assert!(
            !empty.contains(&UiAction::Menu(MenuCommand::StartChallenge(true))),
            "`startChallengeButton.interactable = selectedChampion != null`"
        );
        // A champion picked: its row with "Select Other", and "Start Challenge" works.
        menu.set_champion(champion);
        let picked = actions(&menu);
        assert!(picked.contains(&UiAction::Menu(MenuCommand::StartChallenge(true))));
        assert!(
            picked.contains(&UiAction::Menu(MenuCommand::SelectChampion)),
            "Select Other"
        );
        assert!(!picked.contains(&UiAction::Menu(MenuCommand::StartSimulation)));
    }

    #[test]
    fn the_buttons_beside_the_challenge_list_are_the_challenge_ones() {
        use crate::ui::MenuCommand;
        let Some((runtime, _, atlas)) = window_fixture() else {
            return;
        };
        let (mut menu, _scenario, _champion) =
            crate::menu::MenuState::for_challenge_tests("buttons");
        let sim = atlas
            .texture_of_alias("simulation_icon")
            .expect("the simulation icon is in the atlas");
        let frame = |menu: &crate::menu::MenuState| {
            runtime
                .build_menu_view_frame(menu, &atlas, 1920.0, 1080.0, None)
                .expect("the New Game frame")
        };
        // 70 x 70 `simulation_icon` pictures: the small "launch as a scenario" button is 80, a scenario's own picture is bigger.
        let launchers = |frame: &UiFrame| {
            frame
                .draws
                .iter()
                .filter(|draw| {
                    matches!(draw, UiDraw::Image { texture, size, .. }
                        if *texture == sim && (size.x - 70.0).abs() < 0.5 && (size.y - 70.0).abs() < 0.5)
                })
                .count()
        };
        let titles = |frame: &UiFrame| -> Vec<&'static str> {
            frame.tips.iter().map(|tip| tip.tip.title).collect()
        };
        // The scenario list: "Challenges" and "Create New Scenario" (`showWithScenarios`).
        let scenarios = frame(&menu);
        let scenario_titles = titles(&scenarios);
        for (title, shown) in [
            ("Challenges", true),
            ("Create New Scenario", true),
            ("Scenarios", false),
            ("Create New Challenge", false),
        ] {
            assert_eq!(
                scenario_titles.contains(&title),
                shown,
                "{title}: {scenario_titles:?}"
            );
        }
        // The challenge list: `ScenariosButton` (back to the scenarios, `simulation_icon` over the whole button) and `New Challenge Button`
        // take their places (`showWithChallenges`).
        menu.menu_command(MenuCommand::ToggleChallenges);
        let challenges = frame(&menu);
        let challenge_titles = titles(&challenges);
        for (title, shown) in [
            ("Scenarios", true),
            ("Create New Challenge", true),
            ("Challenges", false),
            ("Create New Scenario", false),
        ] {
            assert_eq!(
                challenge_titles.contains(&title),
                shown,
                "{title}: {challenge_titles:?}"
            );
        }
        assert_eq!(
            launchers(&challenges),
            launchers(&scenarios) + 1,
            "one more 70 x 70 simulation icon: the button back to the scenarios"
        );
        // The tooltip and the picture sit on the same button.
        let back = challenges
            .tips
            .iter()
            .find(|tip| tip.tip.title == "Scenarios")
            .expect("the tooltip of the button back to the scenarios");
        assert!(
            (back.size.x - 70.0).abs() < 0.5 && (back.size.y - 70.0).abs() < 0.5,
            "{:?}",
            back.size
        );
        assert!(
            challenges.draws.iter().any(|draw| matches!(
                draw,
                UiDraw::Image { texture, pos, size, .. }
                    if *texture == sim
                        && pos.distance(back.pos) < 0.01
                        && size.distance(back.size) < 0.01
            )),
            "the picture covers the whole button ({:?} {:?}): {:?}",
            back.pos,
            back.size,
            challenges
                .draws
                .iter()
                .filter(|draw| matches!(draw, UiDraw::Image { texture, .. } if *texture == sim))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_challenge_hides_the_hud_buttons_the_manager_lists_and_closes_the_rows_over_them() {
        let Some((runtime, mut app, atlas)) = native_fixture() else {
            return;
        };
        let frame = |app: &App| runtime.build_frame(app, &atlas, 1920.0, 1080.0);
        let rect_of = |frame: &UiFrame, wanted: UiAction| {
            let (id, _) = frame.actions.iter().find(|(_, action)| *action == wanted)?;
            frame
                .hits
                .iter()
                .find(|hit| hit.0 == *id)
                .map(|(_, pos, size)| (*pos, *size))
        };
        let plain = frame(&app);
        let pen = UiAction::OpenPanel(PanelKind::WorldDynamics);
        let (placer, actions, view) = (
            UiAction::ShowToolbar(0),
            UiAction::ShowToolbar(1),
            UiAction::ShowToolbar(2),
        );
        for action in [pen, placer, actions, view] {
            assert!(
                rect_of(&plain, action).is_some(),
                "{action:?} is a control of the HUD"
            );
        }
        let plain_view = rect_of(&plain, view).expect("view button");
        // The bar-chart button (`InformationPanelButton`) is left of the pen in the top-right row.
        let chart = UiAction::Invoke(ControlCommand::OpenGraphs);
        let plain_chart = rect_of(&plain, chart).expect("the chart button");
        // A challenge begins (its restrictions are what `challenge_is_running` says).
        app.challenge_params = Some(bibites_core::settings::ChallengeParameters::game_defaults());
        let hidden = frame(&app);
        for action in [pen, placer, actions] {
            assert!(
                rect_of(&hidden, action).is_none(),
                "{action:?} is hidden in a challenge"
            );
        }
        // The bottom-left row closes over the two hidden buttons: the view button is where the placer was (x 10).
        let view_now = rect_of(&hidden, view).expect("the view button stays");
        assert!(
            (view_now.0.x - (plain_view.0.x - 120.0)).abs() < 0.5,
            "{view_now:?} {plain_view:?}"
        );
        // The top-right row is right aligned: what was left of the pen moves right by a button and a spacing.
        let chart_now = rect_of(&hidden, chart).expect("the chart button stays");
        assert!(
            (chart_now.0.x - plain_chart.0.x - 65.0).abs() < 0.5,
            "{chart_now:?} {plain_chart:?}"
        );
    }

    #[test]
    fn the_settings_screens_reset_warnings_button_resets_the_warnings() {
        use crate::ui::MenuCommand;
        let Some((runtime, _, atlas)) = native_fixture() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("rustibites-resetwarn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut menu =
            crate::menu::MenuState::new(None, &dir).with_private_settings(&dir.join("s.json"));
        menu.move_selection(4);
        let _ = menu.activate();
        assert_eq!(menu.view(), crate::menu::MenuView::Settings);
        let frame = runtime
            .build_menu_view_frame(&menu, &atlas, 1920.0, 1080.0, None)
            .expect("the settings frame");
        assert!(
            frame
                .actions
                .iter()
                .any(|(_, action)| *action == UiAction::Menu(MenuCommand::ResetWarnings)),
            "the Reset Warnings button resets the warnings (it used to be unavailable)"
        );
        assert!(
            !frame
                .actions
                .iter()
                .any(|(_, action)| *action == UiAction::Menu(MenuCommand::Unavailable(6)))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_disabled_button_tint_is_unitys_default() {
        // `ColorBlock.disabledColor` = (0.784, 0.784, 0.784, 0.502) over an opaque white image.
        assert_eq!(
            disabled_button_tint([255, 255, 255, 255]),
            [200, 200, 200, 128]
        );
        assert_eq!(disabled_button_tint([0, 0, 0, 0]), [0, 0, 0, 0]);
    }

    #[test]
    fn the_title_menus_dialogs_are_not_scaled_by_the_worlds_popup_holder() {
        let Some((runtime, _, atlas)) = native_fixture() else {
            return;
        };
        let build = |scale: f32| {
            runtime
                .dialog_frame(
                    &atlas,
                    1920.0,
                    1080.0,
                    None,
                    "Title",
                    "Text",
                    &["Cancel", "YES"],
                    scale,
                    100,
                )
                .expect("the dialog")
        };
        let title_px = |frame: &UiFrame| {
            frame.draws.iter().find_map(|draw| match draw {
                UiDraw::Text { text, px, .. } if text == "Title" => Some(*px),
                _ => None,
            })
        };
        // The world's holder scales the popup by 1.35: 108 x 40.5 px buttons, 24 pt titles drawn 32.4 px.
        let (frame, world) = build(1.35);
        assert!((world[0][2] - 108.0).abs() < 1e-3 && (world[0][3] - 40.5).abs() < 1e-3);
        assert!(
            (world[0][0] - 703.5).abs() < 1e-3,
            "Cancel at the inner left edge"
        );
        assert!((title_px(&frame).expect("title") - 32.4).abs() < 0.05);
        // The title menu's holder is not scaled: 80 x 30 buttons inside the 400-wide panel (x 760..1160).
        let (frame, menu) = build(1.0);
        assert!((menu[0][2] - 80.0).abs() < 1e-3 && (menu[0][3] - 30.0).abs() < 1e-3);
        assert!(
            (menu[0][0] - 770.0).abs() < 1e-3,
            "Cancel at the inner left edge"
        );
        assert!(
            (menu[1][0] + menu[1][2] - 1150.0).abs() < 1e-3,
            "YES at the inner right edge"
        );
        assert!((title_px(&frame).expect("title") - 24.0).abs() < 0.05);
    }

    #[test]
    fn the_settings_dropdown_rows_open_their_lists_and_a_click_on_an_option_picks_it() {
        use crate::ui::DropdownTarget;
        let Some((runtime, _, atlas)) = native_fixture() else {
            return;
        };
        let Ok(mut fonts) = crate::ui::FontSet::load(&atlas) else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("rustibites-setdrop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut menu =
            crate::menu::MenuState::new(None, &dir).with_private_settings(&dir.join("s.json"));
        menu.move_selection(4);
        let _ = menu.activate();
        assert_eq!(menu.view(), crate::menu::MenuView::Settings);
        let mut checked = 0;
        for tab in 0..3 {
            menu.menu_command(crate::ui::MenuCommand::SettingsTab(tab));
            let frame = runtime
                .build_menu_view_frame(&menu, &atlas, 1920.0, 1080.0, None)
                .expect("the settings frame");
            let rows: Vec<(Vec2, Vec2, DropdownTarget)> = frame
                .actions
                .iter()
                .filter_map(|(id, action)| match action {
                    UiAction::DropdownOpen(target @ DropdownTarget::Setting(_)) => frame
                        .hits
                        .iter()
                        .find(|hit| hit.0 == *id)
                        .map(|hit| (hit.1, hit.2, *target)),
                    _ => None,
                })
                .collect();
            for (pos, size, target) in rows {
                let centre = pos + size * 0.5;
                // A click on the dropdown opens its list under it.
                menu.click_native(&atlas, &mut fonts, 1920.0, 1080.0, Some(&runtime), centre);
                let list = menu.dropdown_list().expect("the click opens the list");
                assert_eq!(list.target, target);
                assert!(
                    (list.anchor[0] - pos.x).abs() < 1.0 && (list.anchor[2] - size.x).abs() < 1.0
                );
                let laid = list.lay_out(Vec2::new(1920.0, 1080.0), 1.0);
                // The list's rows are the topmost hits: a click on the second option picks it.
                let frame = menu.build_frame(&atlas, &mut fonts, 1920.0, 1080.0, Some(&runtime));
                let (row, rect) = laid.rows[1];
                let on_row = Vec2::new(rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5);
                assert_eq!(
                    frame.action_at(on_row),
                    Some(UiAction::DropdownPick(row as u16)),
                    "{target:?}"
                );
                menu.click_native(&atlas, &mut fonts, 1920.0, 1080.0, Some(&runtime), on_row);
                assert!(menu.dropdown_list().is_none(), "the pick closes the list");
                let DropdownTarget::Setting(index) = target else {
                    unreachable!()
                };
                assert_eq!(
                    menu.user_settings().current_choice(usize::from(index)),
                    Some(1),
                    "{target:?}"
                );
                // Opening it again and clicking outside closes it without a pick.
                menu.click_native(&atlas, &mut fonts, 1920.0, 1080.0, Some(&runtime), centre);
                assert!(menu.dropdown_list().is_some());
                menu.click_native(
                    &atlas,
                    &mut fonts,
                    1920.0,
                    1080.0,
                    Some(&runtime),
                    Vec2::new(5.0, 5.0),
                );
                assert!(
                    menu.dropdown_list().is_none(),
                    "a click elsewhere closes it"
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 2,
            "the Settings screens have dropdown rows: {checked}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_new_game_modifiers_open_a_list_instead_of_toggling_a_hand_drawn_one() {
        use crate::ui::DropdownTarget;
        let Some((_, _, _)) = native_fixture() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("rustibites-ngdrop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut menu =
            crate::menu::MenuState::new(None, &dir).with_private_settings(&dir.join("s.json"));
        menu.open_view_for_capture(crate::menu::MenuView::NewGame);
        let dropdown = |path: &str| Node {
            name: "Dropdown".to_owned(),
            path: path.to_owned(),
            ..Node::default()
        };
        let base = "UI/NewGameScenarios/ScenarioSelector/Holder/RightPanels/RightGroup/ParametersPanel/Content/Options";
        // The five modifiers are `basicDropdown`, `basicDropdown (1)` ... `basicDropdown (4)`.
        for (suffix, index) in [
            ("", 0_u8),
            (" (1)", 1),
            (" (2)", 2),
            (" (3)", 3),
            (" (4)", 4),
        ] {
            let action = menu_hit_action(
                &menu,
                &dropdown(&format!(
                    "{base}/left options/basicDropdown{suffix}/Dropdown"
                )),
            );
            assert_eq!(
                action,
                Some(UiAction::DropdownOpen(DropdownTarget::Modifier(index))),
                "basicDropdown{suffix}"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_settings_screens_rows_carry_their_help_texts() {
        use crate::ui::MenuCommand;
        let Some((runtime, _, atlas)) = native_fixture() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("rustibites-settips-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut menu =
            crate::menu::MenuState::new(None, &dir).with_private_settings(&dir.join("s.json"));
        menu.move_selection(4);
        let _ = menu.activate();
        assert_eq!(menu.view(), crate::menu::MenuView::Settings);
        // The three tabs together list the rows' tips: title = the setting's name, text = its help.
        let mut shown: Vec<(&str, &str)> = Vec::new();
        for tab in 0..3 {
            menu.menu_command(MenuCommand::SettingsTab(tab));
            let frame = runtime
                .build_menu_view_frame(&menu, &atlas, 1920.0, 1080.0, None)
                .expect("the settings frame");
            shown.extend(
                frame
                    .tips
                    .iter()
                    .map(|rect| (rect.tip.title, rect.tip.content)),
            );
        }
        let content_of = |title: &str| {
            shown
                .iter()
                .find(|(shown_title, _)| *shown_title == title)
                .map(|(_, content)| *content)
        };
        assert!(shown.len() >= 10, "{} row tips: {shown:?}", shown.len());
        assert_eq!(
            content_of("Show Grid"),
            Some("If enabled, a grid will be displayed at regular intervals")
        );
        assert_eq!(
            content_of("Autosave"),
            Some(
                "This enables autosaves at regular intervals. It's cumbersome, but it allows you to restore the simulation in the event of a crash."
            )
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_librarys_buttons_are_live_and_the_saved_tab_has_its_own_list() {
        use crate::ui::MenuCommand;
        let Some((runtime, _, atlas)) = native_fixture() else {
            return;
        };
        let root = crate::menu::throwaway_library("frame-library");
        let mut menu = crate::menu::MenuState::new(None, &root)
            .with_private_settings(&root.join("s.json"))
            .with_library_root(&root);
        menu.move_selection(3);
        let _ = menu.activate();
        assert_eq!(menu.view(), crate::menu::MenuView::Library);
        let build = |menu: &crate::menu::MenuState| {
            runtime
                .build_menu_view_frame(menu, &atlas, 1920.0, 1080.0, None)
                .expect("the library frame")
        };
        let has = |frame: &UiFrame, wanted: MenuCommand| {
            frame
                .actions
                .iter()
                .any(|(_, action)| *action == UiAction::Menu(wanted))
        };
        let titles = |frame: &UiFrame| -> Vec<String> {
            frame
                .draws
                .iter()
                .filter_map(|draw| match draw {
                    UiDraw::Text { text, .. } if text.ends_with(" Bibites") => Some(text.clone()),
                    _ => None,
                })
                .collect()
        };

        // The Templates tab lists the two templates; the buttons next to them work.
        let frame = build(&menu);
        for wanted in [
            MenuCommand::LibraryTab(0),
            MenuCommand::LibraryTab(1),
            MenuCommand::LibraryReload,
            MenuCommand::LibraryOpenFolder,
        ] {
            assert!(has(&frame, wanted), "{wanted:?}");
        }
        assert!(has(&frame, MenuCommand::SelectTemplate(0)));
        assert!(has(&frame, MenuCommand::SelectTemplate(1)));
        assert!(
            !has(&frame, MenuCommand::SelectTemplate(2)),
            "that one is a saved bibite"
        );
        assert_eq!(titles(&frame), ["Template Bibites"]);
        // `mine` is selected and can be deleted; the official bibite cannot (its button also gets
        // the Button's disabled tint, see `the_disabled_button_tint_is_unitys_default`).
        assert!(has(&frame, MenuCommand::LibraryDelete));
        menu.menu_command(MenuCommand::SelectTemplate(1));
        let frame = build(&menu);
        assert!(!has(&frame, MenuCommand::LibraryDelete));
        // The editor's buttons: the submit button ("Edit Bibite" in a Library opened without a goal), Edit and Create New.
        assert!(has(&frame, MenuCommand::StartTemplate));
        assert!(has(&frame, MenuCommand::EditTemplate));
        assert!(has(&frame, MenuCommand::NewTemplate));
        let words: Vec<&str> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(words.contains(&"Edit Bibite"), "{words:?}");
        assert!(!words.contains(&"Select Bibite"), "{words:?}");

        // The Saved tab lists the saved bibite under its own title.
        menu.menu_command(MenuCommand::LibraryTab(1));
        let frame = build(&menu);
        assert!(has(&frame, MenuCommand::SelectTemplate(2)));
        assert!(!has(&frame, MenuCommand::SelectTemplate(0)));
        assert_eq!(titles(&frame), ["Saved Bibites"]);

        // Delete asks with the file-deletion warning (a popup with its checkbox).
        menu.menu_command(MenuCommand::SelectTemplate(2));
        menu.menu_command(MenuCommand::LibraryDelete);
        assert!(menu.confirm_library_delete());
        let frame = build(&menu);
        assert!(has(&frame, MenuCommand::ConfirmLibraryDelete(true)));
        assert!(has(&frame, MenuCommand::ConfirmLibraryDelete(false)));
        assert!(has(&frame, MenuCommand::ToggleDoNotShowAgain));
        let words: Vec<&str> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(words.contains(&"Deleting Bibite File"), "{words:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_nodes_half_and_quarter_turns_are_read_from_its_local_rotation() {
        let turned = |z: f32, w: f32| {
            let mut node = Node::default();
            node.rect.local_rotation = Quaternion { x: 0.0, y: 0.0, z, w };
            quarter_turns(&node)
        };
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(turned(0.0, 1.0), None, "upright");
        assert_eq!(turned(half, half), Some(1), "+90 degrees (the Library's badges)");
        assert_eq!(turned(1.0, 0.0), Some(2), "half a turn (a closed foldout's arrow)");
        assert_eq!(turned(-1.0, 0.0), Some(2), "-z is the same half turn");
        assert_eq!(turned(-half, half), Some(3), "-90 degrees");
        assert_eq!(turned(0.84, 0.54), None, "a spinner's arbitrary angle is not drawn turned");
    }

    #[test]
    fn the_dynamic_panel_centres_its_labels_on_the_game_fonts_line_boxes() {
        let Some((runtime, mut app, atlas)) = native_fixture() else {
            return;
        };
        let Some(fonts) = crate::text_sdf::SdfFonts::discover(std::path::Path::new(".")) else {
            return;
        };
        app.panels.open(PanelKind::WorldDynamics);
        let frame = runtime.build_frame_with_fonts(&app, &atlas, Some(&fonts), 1920.0, 1080.0);
        let label = |wanted: &str| {
            frame.draws.iter().find_map(|draw| match draw {
                UiDraw::Text { text, pos, .. } if text == wanted => Some(*pos),
                _ => None,
            })
        };
        // Captured in the game (the `Name` boxes are 562 x 40 at 228 and 288 from the top; `text_info` of TMP: Roboto-Bold 25 has a line of 29.297,
        // its baseline sits 8.545 under the box's middle, the line box is centred): the line's top is (40 - 29.297) / 2 under the box's.
        let corpses = label("Corpses").expect("the first toggle's label");
        assert!((corpses.y - (228.0 + (40.0 - 29.296_877) / 2.0)).abs() < 0.05, "{corpses:?}");
        let mutations = label("Prevent Mutations").expect("the second toggle's label");
        assert!((mutations.y - (288.0 + (40.0 - 29.296_877) / 2.0)).abs() < 0.05, "{mutations:?}");
        // The title (Boston Black 36, a 50 high box at 158): a line of 46.8.
        let title = label("Simulation Options").expect("the panel's title");
        assert!((title.y - (158.0 + (50.0 - 46.8) / 2.0)).abs() < 0.05, "{title:?}");
    }

    #[test]
    fn the_worlds_zones_tab_lists_the_live_zones_with_the_games_controls() {
        let Some((runtime, mut app, atlas)) = native_fixture() else {
            return;
        };
        assert!(!app.world.zones.zones.is_empty());
        app.panels.open(PanelKind::WorldDynamics);
        app.dyn_panel.tab = 1;
        let has = |frame: &UiFrame, wanted: UiAction| {
            frame.actions.iter().any(|(_, action)| *action == wanted)
        };
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        // The corner button shows and hides the Preview panel, and says so when hovered.
        assert!(has(
            &frame,
            UiAction::Invoke(ControlCommand::ToggleZonePreview)
        ));
        assert!(
            frame
                .tips
                .iter()
                .any(|rect| rect.tip.title == "Toggle Preview")
        );
        // The Zones header: `ZonesEditorPanel.AddZone` and `DeleteAllZones`.
        assert!(has(&frame, UiAction::Invoke(ControlCommand::AddZone)));
        assert!(has(
            &frame,
            UiAction::Invoke(ControlCommand::RequestClearZones)
        ));
        // The first zone's item, closed: its open bar, its name field, and no delete button yet.
        assert!(has(&frame, UiAction::ZoneOpen(0, 0)));
        assert!(has(&frame, UiAction::ZoneNameEdit(0)));
        assert!(!has(
            &frame,
            UiAction::Invoke(ControlCommand::DeleteZone(0))
        ));
        assert!(
            !frame
                .actions
                .iter()
                .any(|(_, action)| matches!(action, UiAction::ZoneSlide(0, ..))),
            "a closed item shows no controls"
        );
        // Open, it shows the zone's sliders, toggles and dropdowns and its delete button (which
        // asks first); the rows carry the zone settings' help texts.
        app.dyn_panel.open_items.push((0, 0));
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert!(
            frame
                .tips
                .iter()
                .any(|rect| rect.tip.title == "Spawn Pellet of Type"
                    && rect.tip.content == "What type of pellets should be spawned"),
            "the dropdown row's tooltip"
        );
        assert!(
            frame.tips.iter().any(|rect| rect.tip.title == "Biomass "),
            "the slider row's tooltip"
        );
        assert!(has(&frame, UiAction::Invoke(ControlCommand::DeleteZone(0))));
        assert!(
            frame
                .actions
                .iter()
                .any(|(_, action)| matches!(action, UiAction::ZoneSlide(0, ..))),
            "an open item shows its sliders"
        );
        assert!(
            frame.actions.iter().any(|(_, action)| matches!(
                action,
                UiAction::DropdownOpen(crate::ui::DropdownTarget::Zone(0, _))
            )),
            "an open item shows its dropdowns"
        );
        // The Zone Groups header sits below the zones: scrolled down it is there, with its two buttons.
        app.dyn_panel.scroll = app.dyn_panel.max_scroll.get();
        assert!(
            app.dyn_panel.scroll > 0.0,
            "the zone list is longer than the panel"
        );
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert!(has(&frame, UiAction::Invoke(ControlCommand::AddZoneGroup)));
    }

    #[test]
    fn the_worlds_animation_tab_offers_the_creator_the_cards_and_the_graph() {
        use crate::zone_editor as ze;
        let Some((runtime, mut app, atlas)) = native_fixture() else {
            return;
        };
        app.panels.open(PanelKind::WorldDynamics);
        app.dyn_panel.tab = 4;
        let has = |frame: &UiFrame, wanted: UiAction| {
            frame.actions.iter().any(|(_, action)| *action == wanted)
        };
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        // The header: `+` opens the creator; "remove all" is not offered while there are no animators.
        assert!(has(&frame, UiAction::Invoke(ControlCommand::OpenAnimatorCreator)));
        // A click on the `+` (the way the headless harness and the window route it) opens the creator.
        let actions = frame.interact(Vec2::new(970.0, 268.0), true, true, &mut crate::ui::UiInteraction::default());
        app.apply_ui_actions(&actions);
        assert!(app.animator.creator.open, "the + opens the creator: {actions:?}");
        app.animator.creator.open = false;
        // The creator: its preset dropdown, its sliders, Cancel and OK.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenAnimatorCreator)]);
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert!(has(&frame, UiAction::DropdownOpen(crate::ui::DropdownTarget::AnimatorPreset)));
        assert!(has(&frame, UiAction::Invoke(ControlCommand::SubmitAnimatorCreator)));
        assert!(has(&frame, UiAction::Invoke(ControlCommand::CloseAnimatorCreator)));
        for field in crate::animators::creator_fields(crate::animators::Preset::Linear) {
            assert!(
                frame
                    .actions
                    .iter()
                    .any(|(_, action)| matches!(action, UiAction::ZoneSlide(ze::CREATOR_REF, f, ..) if f == field)),
                "creator slider {field}"
            );
        }
        // OK makes the animator: its card is open with its controls.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SubmitAnimatorCreator)]);
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert!(has(&frame, UiAction::Invoke(ControlCommand::RequestClearAnimators)));
        assert!(has(&frame, UiAction::Invoke(ControlCommand::DeleteAnimator(0))));
        assert!(has(&frame, UiAction::ZoneNameEdit(ze::animator_ref(0))));
        assert!(has(&frame, UiAction::ZoneToggle(ze::animator_ref(0), ze::ANIM_REPEAT)));
        assert!(has(&frame, UiAction::ZoneReset(ze::animator_ref(0), ze::ANIM_START)));
        assert!(has(&frame, UiAction::DropdownOpen(crate::ui::DropdownTarget::AnimatorBase(0))));
        assert!(
            frame
                .actions
                .iter()
                .any(|(_, action)| matches!(action, UiAction::ZoneSlide(r, f, ..) if *r == ze::animator_ref(0) && *f == ze::ANIM_START)),
            "the start slider"
        );
        assert!(
            frame.actions.iter().any(|(_, action)| matches!(action, UiAction::AnimGraph(0, ..))),
            "the graph's plot"
        );
        // A base item and a picked point bring their controls.
        assert!(app.animator_dropdown_pick(crate::ui::DropdownTarget::AnimatorBase(0), 1));
        app.animator.point = Some((0, 1));
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert!(has(&frame, UiAction::Invoke(ControlCommand::RemoveAnimatorBase(0, 0))));
        assert!(has(&frame, UiAction::DropdownOpen(crate::ui::DropdownTarget::AnimatorInterpolation(0))));
        assert!(has(&frame, UiAction::DynEditValue(crate::dyn_panel::FieldRef::Zone(ze::animator_ref(0), ze::ANIM_POINT_TIME))));
    }

    /// The report "I can't create a settings animator", frame by frame the way the window runs it: a click is a press on one frame and the release on a LATER
    /// one, built after the app moved on (`UiFrame::interact` fires only when the same widget is under the pointer at both). The Information panel was open:
    /// it stayed over the settings panel and over the creator's OK button, and a click on OK fell on the panel. `DynamicSettingsPanel.onOpen` deactivates
    /// `QuickStatsHolder` (the panel and the Info tab), so with the pen pressed nothing of it is left to cover or take a click; and OK creates the
    /// animator for every preset, also when the Information panel is dismissed first.
    #[test]
    fn the_creators_ok_button_creates_the_animator_with_the_information_panel_around() {
        let Some((runtime, mut app, atlas)) = native_fixture() else {
            return;
        };
        let (width, height) = (1920.0, 1080.0);
        app.set_viewport(width, height);
        let build = |app: &App| {
            // The window's order: the runtime frame, the extra panels over it, an open dropdown list over those.
            let mut frame = runtime.build_frame_with_fonts(app, &atlas, None, width, height);
            crate::hud::draw_extra_panels(&mut frame, app, &atlas, None, width, height, true);
            if let Some(open) = app.dropdown_open.as_ref() {
                crate::dropdown::emit(&mut frame, &atlas, None, open, Vec2::new(width, height), app.cursor);
            }
            frame
        };
        let has = |frame: &UiFrame, wanted: UiAction| frame.actions.iter().any(|(_, action)| *action == wanted);
        let rect_of = |frame: &UiFrame, wanted: UiAction| -> (Vec2, Vec2) {
            let (id, _) = frame
                .actions
                .iter()
                .find(|(_, action)| *action == wanted)
                .unwrap_or_else(|| panic!("{wanted:?} is not in the frame"));
            let (_, pos, size) = frame.hits.iter().find(|hit| hit.0 == *id).expect("its rectangle");
            (*pos, *size)
        };
        // One click on the centre of a control: what is on top there must be the control (else the click lands on what covers it).
        let click_on = |app: &mut App, wanted: UiAction| {
            let pressed = build(app);
            let (pos, size) = rect_of(&pressed, wanted);
            let at = pos + size * 0.5;
            assert_eq!(pressed.action_at(at), Some(wanted), "{wanted:?} is covered at {at:?}");
            // `remember_click_rect`: a dropdown's list hangs from the control that was clicked.
            app.click_rect = [pos.x / pressed.scale, pos.y / pressed.scale, size.x / pressed.scale, size.y / pressed.scale];
            pressed.interact(at, true, false, &mut app.interaction);
            app.advance(1.0 / 60.0);
            let actions = build(app).interact(at, false, true, &mut app.interaction);
            assert!(actions.contains(&wanted), "the release did not fire {wanted:?}: {actions:?}");
            app.apply_ui_actions(&actions);
        };
        let (open_info, close_info) = (UiAction::Invoke(ControlCommand::OpenInformation), UiAction::Invoke(ControlCommand::InformationClose));
        let pen = UiAction::OpenPanel(PanelKind::WorldDynamics);
        let create = |app: &mut App, preset: usize| {
            click_on(app, UiAction::DynTab(4));
            click_on(app, UiAction::Invoke(ControlCommand::OpenAnimatorCreator));
            assert!(app.animator.creator.open, "the + opens the creator");
            if preset > 0 {
                click_on(app, UiAction::DropdownOpen(crate::ui::DropdownTarget::AnimatorPreset));
                click_on(app, UiAction::DropdownPick(preset as u16));
                assert_eq!(app.animator.creator.preset, crate::animators::Preset::ALL[preset], "the list's pick is the preset");
            }
            let before = app.world.settings_changers.len();
            click_on(app, UiAction::Invoke(ControlCommand::SubmitAnimatorCreator));
            assert!(!app.animator.creator.open, "OK closes the creator");
            assert_eq!(app.world.settings_changers.len(), before + 1, "OK creates the animator: {}", app.last_action);
        };
        // The Info tab opens the Information panel; the pen then opens the settings panel, and `QuickStatsHolder` goes inactive with it.
        assert!(has(&build(&app), open_info), "the Info tab is there");
        click_on(&mut app, open_info);
        assert!(app.panels.is_open(PanelKind::Information) && has(&build(&app), close_info), "the Information panel is up");
        click_on(&mut app, pen);
        assert!(app.panels.is_open(PanelKind::WorldDynamics) && app.quick_stats_hidden());
        let with_pen = build(&app);
        assert!(!has(&with_pen, close_info), "no Information panel over the settings panel");
        assert!(!has(&with_pen, open_info), "no Info tab in the corner of the settings panel");
        // The first animator, the Linear preset, with the Information panel still open underneath (it was never dismissed).
        create(&mut app, 0);
        assert!(app.panels.is_open(PanelKind::Information), "only inactive meanwhile");
        // The settings panel closes (its X): `QuickStatsHolder` is active again, the Information panel is as it was, and is dismissed with its X.
        click_on(&mut app, UiAction::ClosePanel);
        assert!(!app.panels.is_open(PanelKind::WorldDynamics) && !app.quick_stats_hidden());
        let without_pen = build(&app);
        assert!(has(&without_pen, close_info) && !has(&without_pen, open_info), "the Information panel is back, the Info tab stays hidden");
        click_on(&mut app, close_info);
        assert!(!app.panels.is_open(PanelKind::Information));
        assert!(has(&build(&app), open_info), "the Info tab is back with the panel closed");
        // The other presets, with the dropdown in between, on the Information-free screen.
        click_on(&mut app, pen);
        create(&mut app, 1);
        create(&mut app, 2);
        assert_eq!(app.world.settings_changers.len(), 3);
    }

    /// Every click area of the window's frame has an id of its own (the runtime frame, the panels over it, an open dropdown list), in the states of the
    /// settings flow: `UiFrame::interact` answers with every action of the id it hit, so a shared id fires a second control's action.
    #[test]
    fn the_windows_frame_gives_every_click_area_an_id_of_its_own() {
        let Some((runtime, mut app, atlas)) = native_fixture() else {
            return;
        };
        let (width, height) = (1920.0, 1080.0);
        app.set_viewport(width, height);
        let build = |app: &App| {
            let mut frame = runtime.build_frame_with_fonts(app, &atlas, None, width, height);
            crate::hud::draw_extra_panels(&mut frame, app, &atlas, None, width, height, true);
            if let Some(open) = app.dropdown_open.as_ref() {
                crate::dropdown::emit(&mut frame, &atlas, None, open, Vec2::new(width, height), app.cursor);
            }
            frame
        };
        let mut problems = Vec::new();
        let mut check = |app: &App, what: &str| {
            let frame = build(app);
            let shared = frame.shared_ids();
            eprintln!("IDS {what}: {} click areas, {} shared ids", frame.hits.len(), shared.len());
            for id in shared {
                let actions: Vec<String> = frame.actions.iter().filter(|(other, _)| *other == id).map(|(_, action)| format!("{action:?}")).collect();
                problems.push(format!("{what}: id {id} carries {actions:?}"));
            }
        };
        check(&app, "the world");
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenInformation)]);
        check(&app, "the Information panel");
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::WorldDynamics)]);
        for tab in 0..=4 {
            app.dyn_panel.tab = tab;
            check(&app, &format!("the settings panel, tab {tab}"));
        }
        app.dyn_panel.tab = 4;
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenAnimatorCreator)]);
        check(&app, "the animator creator");
        for preset in 0..3 {
            app.animator.creator.preset = crate::animators::Preset::ALL[preset];
            app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SubmitAnimatorCreator)]);
            app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenAnimatorCreator)]);
        }
        check(&app, "the animator creator over three animators");
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::CloseAnimatorCreator)]);
        check(&app, "three animators");
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::Graphs)]);
        check(&app, "the Graphs page");
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenEscapeMenu)]);
        check(&app, "the pause menu over the Graphs page");
        assert!(problems.is_empty(), "{problems:#?}");
    }

    /// The settings panel is flush with the right edge of the canvas (the window over the UI scale), which is wider than 1920 reference px in a window that is
    /// wider than 16:9. Its click areas were laid out for 1920 x 1080 and stayed where the 16:9 panel has its buttons while the panel moved: in any window that
    /// is not 16:9 (a maximised one, 16:10, ultra-wide) the visible tabs, Save, Close and the animator creator's OK and Cancel were somewhere else than where
    /// they answered clicks (the sliders' tracks too), and the creator was drawn lower by what the canvas has beyond 1080 reference px in height.
    /// The capture's own window (1920 x 1080) is the reference: every click area, and every text of the panel, is where it is there moved right by what the
    /// canvas has beyond 1920 reference px (nothing moves down), all of it times the UI scale. (The test atlas has no sprites, so the texts are what shows
    /// where the panel is drawn; the pictures of a 1600 x 900 window clicked where the buttons are drawn are in docs/PARITY_REMAINING.md.)
    #[test]
    fn the_settings_panels_buttons_answer_clicks_where_they_are_drawn_in_any_window() {
        let Some((runtime, mut app, atlas)) = native_fixture() else {
            return;
        };
        app.panels.open(PanelKind::WorldDynamics);
        app.dyn_panel.tab = 4;
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenAnimatorCreator)]);
        let build = |app: &mut App, width: f32, height: f32| {
            app.set_viewport(width, height);
            runtime.build_frame_with_fonts(app, &atlas, None, width, height)
        };
        let rect_of = |frame: &UiFrame, wanted: UiAction| -> (Vec2, Vec2) {
            let (id, _) = frame
                .actions
                .iter()
                .find(|(_, action)| *action == wanted)
                .unwrap_or_else(|| panic!("{wanted:?} is not in the frame"));
            let (_, pos, size) = frame.hits.iter().find(|hit| hit.0 == *id).expect("its rectangle");
            (*pos, *size)
        };
        let text_at = |frame: &UiFrame, wanted: &str| -> Vec2 {
            frame
                .draws
                .iter()
                .find_map(|draw| match draw {
                    UiDraw::Text { text, pos, .. } if text.trim() == wanted => Some(*pos),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the text {wanted:?} is not drawn"))
        };
        let near = |a: Vec2, b: Vec2, tolerance: f32| (a.x - b.x).abs() < tolerance && (a.y - b.y).abs() < tolerance;
        let buttons = [
            ("the first tab", UiAction::DynTab(0)),
            ("the Animation tab", UiAction::DynTab(4)),
            ("Save", UiAction::DynSaveScenario),
            ("Close", UiAction::ClosePanel),
            ("the creator's OK", UiAction::Invoke(ControlCommand::SubmitAnimatorCreator)),
            ("the creator's Cancel", UiAction::Invoke(ControlCommand::CloseAnimatorCreator)),
        ];
        let labels = ["Settings Animator", "Animator Presets", "Preset", "Start Time", "OK", "Cancel"];
        let reference = build(&mut app, 1920.0, 1080.0);
        assert_eq!(reference.scale, 1.0);
        let button_rects: Vec<(Vec2, Vec2)> = buttons.iter().map(|(_, wanted)| rect_of(&reference, *wanted)).collect();
        let label_places: Vec<Vec2> = labels.iter().map(|label| text_at(&reference, label)).collect();
        for (width, height) in [(1920.0_f32, 1080.0_f32), (1900.0, 1000.0), (2560.0, 1080.0), (1280.0, 720.0), (1600.0, 1200.0), (1920.0, 1200.0), (1500.0, 900.0)] {
            let frame = build(&mut app, width, height);
            let scale = frame.scale;
            let dx = width / scale - 1920.0;
            let moved = |at: Vec2| Vec2::new((at.x + dx) * scale, at.y * scale);
            for ((label, wanted), (ref_pos, ref_size)) in buttons.iter().zip(&button_rects) {
                let (pos, size) = rect_of(&frame, *wanted);
                assert!(
                    near(pos, moved(*ref_pos), 0.75) && near(size, *ref_size * scale, 0.75),
                    "{label} answers clicks at {pos:?} ({size:?}) in a {width} x {height} window, not at {:?} ({:?})",
                    moved(*ref_pos),
                    *ref_size * scale
                );
            }
            // What is drawn moves with it.
            for (label, ref_at) in labels.iter().zip(&label_places) {
                let at = text_at(&frame, label);
                assert!(near(at, moved(*ref_at), 6.0), "the text {label:?} is drawn at {at:?} in a {width} x {height} window, not at {:?}", moved(*ref_at));
            }
            // The OK and Cancel texts start on their buttons.
            for (label, wanted) in [("OK", UiAction::Invoke(ControlCommand::SubmitAnimatorCreator)), ("Cancel", UiAction::Invoke(ControlCommand::CloseAnimatorCreator))] {
                let (pos, size) = rect_of(&frame, wanted);
                let at = text_at(&frame, label);
                assert!(
                    at.x >= pos.x && at.x < pos.x + size.x && at.y >= pos.y && at.y < pos.y + size.y,
                    "the text {label:?} is drawn at {at:?}, off its button ({pos:?}, {size:?}) in a {width} x {height} window"
                );
            }
            // The creator's sliders carry tracks inside the creator (954 .. 1559 reference px in the capture), moved with the panel.
            let tracks: Vec<(u16, u16)> = frame
                .actions
                .iter()
                .filter_map(|(_, action)| match action {
                    UiAction::ZoneSlide(reference, _, x0, x1) if *reference == crate::zone_editor::CREATOR_REF => Some((*x0, *x1)),
                    _ => None,
                })
                .collect();
            assert!(!tracks.is_empty(), "the creator has sliders");
            for (x0, x1) in tracks {
                assert!(
                    f32::from(x0) >= 954.0 + dx - 1.0 && f32::from(x1) <= 1559.0 + dx + 1.0 && x0 < x1,
                    "a creator slider's track {x0}..{x1} is not inside the creator ({} .. {}) in a {width} x {height} window",
                    954.0 + dx,
                    1559.0 + dx
                );
            }
        }
    }

    #[test]
    fn the_worlds_controls_carry_their_tooltips() {
        let Some((runtime, mut app, atlas)) = native_fixture() else {
            return;
        };
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        let title_of = |frame: &UiFrame, title: &str| {
            frame
                .tips
                .iter()
                .find(|rect| rect.tip.title == title)
                .copied()
                .unwrap_or_else(|| panic!("no tooltip rectangle for {title}"))
        };
        // The pause button: the rectangle is where the button is drawn (top centre).
        let pause = title_of(&frame, "Toggle Pause [SpaceBar]");
        assert!(pause.size.x > 20.0 && pause.size.y > 20.0, "{pause:?}");
        assert!(
            pause.pos.y < 100.0 && pause.pos.x > 600.0 && pause.pos.x < 900.0,
            "{pause:?}"
        );
        // The top-right buttons of the layer-0 canvas.
        assert!(
            title_of(&frame, "Dynamic Settings").pos.x > 1400.0,
            "the dynamic settings button is at the right"
        );
        // A panel's own controls get theirs once the panel is up: the dynamic panel's tabs.
        app.panels.open(PanelKind::WorldDynamics);
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        let zones_tab = title_of(&frame, "Zone Parameters");
        assert!(zones_tab.pos.x > 900.0, "{zones_tab:?}");
        // The settings rows of the generated tabs show the setting's name and help text when the
        // pointer rests on the label (`SettingSlider.UpdateText(Name, HelperText)`).
        app.dyn_panel.tab = 1;
        let frame_zones = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        let biomass = title_of(&frame_zones, "Global Biomass Density");
        assert!(
            biomass
                .tip
                .content
                .starts_with("Defines the default amount of biomass")
        );
        assert!(
            biomass.pos.x > 900.0 && biomass.size.x > 100.0,
            "{biomass:?}"
        );
        app.dyn_panel.tab = 0;
        // Moving the pointer onto it, and resting, shows the box for that control.
        let over = zones_tab.pos + zones_tab.size * 0.5;
        let over_ui = frame.widget_at(over).is_some();
        app.update_tooltip(&frame, over, over_ui, 0.6);
        assert!(
            matches!(app.tooltip.shown(), Some((crate::tooltip::Target::Control(tip), _)) if tip.title == "Zone Parameters")
        );
    }

    #[test]
    fn the_worlds_energy_tab_has_the_four_material_blocks() {
        let Some((runtime, mut app, atlas)) = native_fixture() else {
            return;
        };
        app.panels.open(PanelKind::WorldDynamics);
        app.dyn_panel.tab = 3;
        // The Materials Parameters group is the last one: scroll down to it.
        let _ = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        app.dyn_panel.scroll = app.dyn_panel.max_scroll.get();
        assert!(app.dyn_panel.scroll > 0.0);
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        let gears: Vec<UiAction> = frame
            .actions
            .iter()
            .filter(|(_, action)| matches!(action, UiAction::DynSub(_)))
            .map(|(_, action)| *action)
            .collect();
        assert_eq!(gears.len(), 4, "one gear per material: {gears:?}");
        assert!(
            !frame
                .actions
                .iter()
                .any(|(_, action)| matches!(action, UiAction::MaterialSlide(..))),
            "closed blocks show no sliders"
        );
        // Opening a block shows its parameter sliders and its decay switch (scrolled to the end
        // again, now that the content is longer).
        app.apply_ui_actions(&[gears[0]]);
        let _ = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        app.dyn_panel.scroll = app.dyn_panel.max_scroll.get();
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert!(
            frame
                .actions
                .iter()
                .any(|(_, action)| matches!(action, UiAction::MaterialSlide(0, ..))),
            "an open block shows its sliders"
        );
        assert!(
            frame
                .actions
                .iter()
                .any(|(_, action)| matches!(action, UiAction::MaterialToggle(0))),
            "an open block shows its decay switch"
        );
    }

    #[test]
    fn default_runtime_hud_a_b_and_determinism() {
        let Some((runtime, app, mut atlas)) = default_fixture() else {
            return;
        };
        // The runtime dump references a few PNGs the UI atlas skips; the loader
        // must decode them from the sprite manifest before frame building.
        let extra_textures = runtime
            .load_sprite_textures(&mut atlas)
            .expect("decode runtime sprite textures");
        assert!(extra_textures >= 4, "extra textures = {extra_textures}");
        let first = runtime.build_frame(&app, &atlas, 1280.0, 800.0);
        let second = runtime.build_frame(&app, &atlas, 1280.0, 800.0);
        let curated = crate::hud::build(&app, &atlas, 1280.0, 800.0);
        assert!(!first.draws.is_empty(), "runtime frame has no draws");
        assert!(
            !first.hits.is_empty(),
            "runtime frame has no interactive hits"
        );
        // Determinism: two builds from the same state are byte-for-byte equal.
        assert_eq!(
            format!("{:?}", first.draws),
            format!("{:?}", second.draws),
            "runtime draw list is not deterministic"
        );
        assert_eq!(first.hits, second.hits);
        // TMP multi-line labels must become separate single-line draws: the
        // software text path does not handle embedded newlines.
        assert!(
            !first.draws.iter().any(|draw| matches!(
                draw,
                UiDraw::Text { text, .. } if text.contains('\n')
            )),
            "a UiDraw::Text still contains an embedded newline"
        );
        assert!(
            first
                .draws
                .iter()
                .filter(|draw| matches!(draw, UiDraw::Text { .. }))
                .count()
                >= 3,
            "expected the split time block (Time:/fps/sim lines) in the HUD"
        );
        assert!(
            first.draws.iter().any(|draw| matches!(
                draw,
                UiDraw::Text { text, .. } if text.starts_with("Time: ")
            )),
            "expected a live time label"
        );
        // A/B: the runtime layout must differ from the curated HUD layout.
        assert_ne!(
            format!("{:?}", first.draws),
            format!("{:?}", curated.draws),
            "runtime HUD equals curated HUD"
        );
    }

    #[test]
    fn committed_dump_has_save_state_inspector_and_ruler_value() {
        let Ok(Some(runtime)) = RuntimeUi::discover() else {
            eprintln!("skipping: runtime UI dump absent");
            return;
        };
        assert!(
            runtime.inspector_session.is_some(),
            "save-state dump should carry the expanded-inspector session"
        );
        assert!(
            runtime.ruler_value.is_some(),
            "save-state dump should expose the measured ruler value"
        );
        let inspector = runtime.inspector_session.as_ref().unwrap();
        assert!(inspector.node_count > runtime.session.node_count);
        assert!(inspector.core_panel_paths.len() >= runtime.session.core_panel_paths.len());
    }

    #[test]
    fn save_state_ruler_and_expanded_inspector_frames() {
        let Some((runtime, mut app, mut atlas)) = save_fixture() else {
            return;
        };
        runtime
            .load_sprite_textures(&mut atlas)
            .expect("decode runtime sprite textures");
        // Match the game HUD reference camera (fit world, ortho 2080).
        app.set_ortho_size(2080.0, 1080.0);
        let default_frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert!(!runtime.uses_inspector_session(&app));
        // Procedural ruler: measured value + bracket are drawn even though the
        // dump has no ruler sprites.
        assert!(
            default_frame.draws.iter().any(|draw| matches!(
                draw,
                UiDraw::Text { text, .. } if text == "200"
            )),
            "procedural ruler measurement missing"
        );
        assert!(
            default_frame
                .draws
                .iter()
                .filter(|draw| matches!(
                    draw,
                    UiDraw::Rect { color, .. } if *color == [255, 255, 255, 255]
                ))
                .count()
                >= 3,
            "ruler bracket should be three white lines"
        );
        // Selecting a bibite on the Stats tab switches to the expanded session.
        app.select_first();
        app.open_bibite_panel(InspectorTab::Stats, true);
        assert!(runtime.uses_inspector_session(&app));
        let inspector_frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert!(
            inspector_frame.draws.len() > default_frame.draws.len(),
            "expanded-inspector frame should add panel draws"
        );
        assert!(
            inspector_frame
                .draws
                .iter()
                .any(|draw| matches!(draw, UiDraw::Text { text, .. } if text.contains("Species"))),
            "expanded Stats panel text missing"
        );
    }

    #[test]
    fn default_scenario_uses_fresh_spawner() {
        let path = workspace_path("testdata/real/Default.zip");
        if !path.is_file() {
            eprintln!("skipping: testdata/real/Default.zip absent");
            return;
        }
        let Ok(loaded) = crate::app::LoadedWorld::load(Some(&path), 12345, 0, 0) else {
            return;
        };
        // Fresh-scenario semantics: one bibite immediately, `virginSpawnRate`
        // grows the scenario to its minimum (10) over ~9 s.
        assert_eq!(loaded.spawned, 1, "scenario should start with one bibite");
        let mut app = App::new(loaded, Vec2::new(1280.0, 800.0), None);
        app.refresh_overlay(0.0);
        assert_eq!(app.hud.population, 1);
        for _ in 0..1200 {
            app.advance(1.0 / 60.0);
        }
        app.refresh_overlay(0.0);
        assert_eq!(app.world_tick(), 600);
        assert_eq!(app.hud.population, 10, "spawner should reach the minimum");
    }

    #[test]
    fn inspector_tab_sessions_build_distinct_deterministic_frames() {
        let Some((runtime, mut app, mut atlas)) = save_fixture() else {
            return;
        };
        for tab in [
            InspectorTab::Genes,
            InspectorTab::Biology,
            InspectorTab::Brain,
        ] {
            assert!(
                runtime.tab_session(tab).is_some(),
                "missing runtime session for {tab:?}"
            );
        }
        runtime
            .load_sprite_textures(&mut atlas)
            .expect("decode runtime sprite textures");
        app.set_ortho_size(2080.0, 1080.0);
        app.select_first();
        let mut seen = Vec::new();
        for tab in [
            InspectorTab::Genes,
            InspectorTab::Biology,
            InspectorTab::Brain,
        ] {
            app.inspector_tab = tab;
            app.inspector_open = true;
            assert!(runtime.uses_inspector_session(&app));
            let first = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
            let second = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
            assert!(!first.draws.is_empty());
            assert_eq!(format!("{:?}", first.draws), format!("{:?}", second.draws));
            seen.push(format!("{:?}", first.draws));
        }
        assert_ne!(seen[0], seen[1], "Genes and Biology frames are identical");
        assert_ne!(seen[0], seen[2], "Genes and Brain frames are identical");
        assert_ne!(seen[1], seen[2], "Biology and Brain frames are identical");
    }

    #[test]
    fn runtime_control_registry_and_generic_hits_cover_every_active_control() {
        let Some((runtime, mut app, mut atlas)) = save_fixture() else {
            return;
        };
        runtime
            .load_sprite_textures(&mut atlas)
            .expect("decode runtime sprite textures");
        app.set_ortho_size(2080.0, 1080.0);

        let all = runtime.controls(1920.0, 1080.0);
        assert!(!all.is_empty(), "runtime control registry is empty");
        for control in &all {
            assert!(control.id != 0);
            assert!(control.rect.iter().all(|value| value.is_finite()));
        }
        for session in [
            "default_world",
            "default_world_inspector",
            "default_world_genes",
            "default_world_biology",
            "default_world_brain",
            "main_menu_title",
            "main_menu_settings",
        ] {
            assert!(
                all.iter().any(|control| control.session == session),
                "registry is missing active controls for {session}"
            );
        }

        // Registry ids are stable and unique within a session.
        let default_controls: Vec<_> = all
            .iter()
            .filter(|control| control.session == "default_world")
            .cloned()
            .collect();
        let mut ids: Vec<_> = default_controls.iter().map(|control| control.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), default_controls.len());

        // Known controls still map to their concrete actions.
        let pause = default_controls
            .iter()
            .find(|control| control.name == "PauseButton")
            .expect("PauseButton in registry");
        assert_eq!(pause.action, Some(UiAction::TogglePause));
        let play = default_controls
            .iter()
            .find(|control| control.name == "PlayButton")
            .expect("PlayButton in registry");
        assert_eq!(play.action, Some(UiAction::SetSpeedSlider(1.0)));
        let settings = default_controls
            .iter()
            .find(|control| control.name == "SettingsButton")
            .expect("SettingsButton in registry");
        // The gear opens the pause menu (`EscapePanel.OpenPanel`), not a settings screen.
        assert_eq!(
            settings.action,
            Some(UiAction::Invoke(ControlCommand::OpenEscapeMenu))
        );

        // Every active control in each selected-tab session yields a hit rect.
        app.select_first();
        for (tab, session) in [
            (InspectorTab::Stats, "default_world_inspector"),
            (InspectorTab::Genes, "default_world_genes"),
            (InspectorTab::Biology, "default_world_biology"),
            (InspectorTab::Brain, "default_world_brain"),
        ] {
            app.inspector_tab = tab;
            app.inspector_open = true;
            let count = all
                .iter()
                .filter(|control| control.session == session)
                .count();
            assert!(count > 0, "empty active-control session {session}");
            let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
            // The speed slider's draggable panel and its number (the edit button that opens the typed field) are extra, synthetic hits (not controls).
            let control_hits = frame
                .actions
                .iter()
                .filter(|(_, action)| {
                    !matches!(
                        action,
                        UiAction::SpeedSlide(..)
                            | UiAction::DynEditValue(crate::dyn_panel::FieldRef::TimeSpeed)
                    )
                })
                .count();
            assert_eq!(control_hits, count, "hit coverage mismatch for {tab:?}");
            assert!(
                frame
                    .actions
                    .iter()
                    .any(|(_, action)| matches!(action, UiAction::ActivateControl(_))),
                "unmapped controls should stay exposed as ActivateControl hits for {tab:?}"
            );
        }

        // Menu captures expose every button, including the not-yet-functional
        // rows; counts follow the current atlas rather than a frozen literal.
        let title_controls = runtime
            .session_controls("main_menu_title", 1920.0, 1080.0)
            .len();
        let menu = runtime
            .build_menu_frame(false, &atlas, 1920.0, 1080.0, None)
            .expect("menu title session");
        assert_eq!(menu.hits.len(), title_controls);
        let settings_controls = runtime
            .session_controls("main_menu_settings", 1920.0, 1080.0)
            .len();
        let settings = runtime
            .build_menu_frame(true, &atlas, 1920.0, 1080.0, None)
            .expect("menu settings session");
        assert!(settings.hits.len() >= settings_controls);
    }

    #[test]
    fn challenge_workshop_lineage_sessions_register_controls() {
        let Some((runtime, _app, _atlas)) = save_fixture() else {
            return;
        };
        let sessions = [
            "challenge_selector",
            "challenge_scenario_panel",
            "challenge_champion_selection",
            "challenge_parameters_editor",
            "workshop_panel",
            "workshop_edit_shared",
            "lineage_species_panel",
            "lineage_species_tabs",
            "lineage_species_info",
            "lineage_species_genes",
            "lineage_species_brain",
        ];
        for session in sessions {
            let controls = runtime.session_controls(session, 1920.0, 1080.0);
            assert!(!controls.is_empty(), "{session} has no registered controls");
            let mut ids = std::collections::HashSet::new();
            for control in &controls {
                assert_eq!(control.id, control_id(session, &control.path));
                assert!(ids.insert(control.id), "duplicate id in {session}");
                assert!(control.rect[2] >= 0.0 && control.rect[3] >= 0.0);
            }
            let frame = runtime
                .build_named_session_frame(session, &_atlas, 1920.0, 1080.0)
                .unwrap_or_else(|| panic!("missing frame for {session}"));
            assert!(
                frame.hits.len() + 1 >= controls.len(),
                "hit coverage for {session}: {} vs {}",
                frame.hits.len(),
                controls.len()
            );
        }

        let workshop = runtime.session_controls("workshop_panel", 1920.0, 1080.0);
        for name in [
            "EditSharedButon",
            "refreshButton",
            "OpenWorkshopButton",
            "UnsubscribeButton",
            "ShareNewButon",
        ] {
            assert!(
                workshop.iter().any(|control| control.name == name),
                "workshop_panel missing {name}"
            );
        }
        let edit = runtime.session_controls("workshop_edit_shared", 1920.0, 1080.0);
        for name in [
            "DeleteItemButton",
            "UpdateFileButton",
            "Submit Button",
            "ChangePreviewButton",
        ] {
            assert!(
                edit.iter().any(|control| control.name == name),
                "workshop_edit_shared missing {name}"
            );
        }
        let subscribed = "UI/WorkshopItemsPanel/Holder/SubscribedList/body/Scroll View";
        assert_ne!(
            workshop_row_id("workshop_panel", subscribed, 0),
            workshop_row_id("workshop_panel", subscribed, 1)
        );
        assert_eq!(
            workshop_row_id("workshop_panel", subscribed, 0),
            dynamic_row_id("workshop_panel", subscribed, 0)
        );
    }

    #[test]
    fn native_registry_rects_match_screen_bbox_and_representative_actions() {
        use std::collections::HashMap;
        use std::path::Path;
        let Ok(runtime) =
            RuntimeUi::load_native(Path::new("assets/game/ui_runtime_native.json.gz"))
        else {
            return;
        };
        // The window parses the HUD sessions at start and the others on first use; the audit covers all sixty.
        let controls = runtime.controls_all(1920.0, 1080.0);
        assert!(
            controls.len() >= 1400,
            "unexpected native registry size: {}",
            controls.len()
        );
        assert_eq!(runtime.session_names().len(), 60);
        assert!(
            controls
                .iter()
                .find(|control| {
                    control.session == "main_menu_title"
                        && control.path.ends_with("/ButtonsPanel/Continue")
                })
                .is_some_and(
                    |control| control.action == Some(UiAction::Invoke(ControlCommand::QuickLoad))
                )
        );
        assert!(
            controls
                .iter()
                .find(|control| {
                    control.session == "main_menu_title"
                        && control.path.ends_with("/ButtonsPanel/Quit")
                })
                .is_some_and(|control| control.action == Some(UiAction::Quit))
        );
        assert!(controls.iter().any(|control| {
            control.session == "main_menu_settings_graphics"
                && control.kind == ControlKind::Slider
                && control
                    .value
                    .as_ref()
                    .is_some_and(|value| value.value.is_some())
        }));
        assert!(controls.iter().any(|control| {
            control.session == "main_menu_settings_graphics"
                && control.kind == ControlKind::Dropdown
                && control
                    .value
                    .as_ref()
                    .is_some_and(|value| !value.options.is_empty())
        }));

        // Every registry rect must equal the node's native v2 screen_bbox, the
        // same geometry consumed by the frame composer and hit rects.
        let mut native_rects: HashMap<String, Vec<[f32; 4]>> = HashMap::new();
        fn walk(node: &Node, out: &mut HashMap<String, Vec<[f32; 4]>>) {
            if !node.active_in_hierarchy {
                return;
            }
            if let Some([x0, y0, x1, y1]) = node.native_rect {
                out.entry(node.path.clone()).or_default().push([
                    x0,
                    y0,
                    (x1 - x0).max(0.0),
                    (y1 - y0).max(0.0),
                ]);
            }
            for child in &node.children {
                walk(child, out);
            }
        }
        for name in runtime.session_names() {
            let Some(session) = runtime.session_by_name(&name) else {
                continue;
            };
            for root in active_canvas_roots(session) {
                walk(root, &mut native_rects);
            }
        }
        for control in &controls {
            let Some(rects) = native_rects.get(&control.path) else {
                panic!(
                    "control has no native screen_bbox: {}/{}",
                    control.session, control.path
                );
            };
            assert!(
                rects.iter().any(|rect| rect
                    .iter()
                    .zip(control.rect.iter())
                    .all(|(a, b)| (a - b).abs() <= 0.1)),
                "hit rect != native screen_bbox: {}/{} {:?} vs {:?}",
                control.session,
                control.path,
                control.rect,
                rects
            );
        }
    }

    #[test]
    fn toolbar_zones_cover_the_buttons_and_grow_with_the_open_panel() {
        let Some((runtime, mut app, _atlas)) = default_fixture() else {
            return;
        };
        let closed = runtime.toolbar_zones(&app, 1920.0, 1080.0);
        for (index, zone) in closed.iter().enumerate() {
            let (pos, size) = zone.expect("a button zone");
            assert!(size.x > 20.0 && size.y > 20.0, "button {index}: {size:?}");
            // The buttons sit in the bottom-left corner.
            assert!(pos.x < 200.0 && pos.y > 900.0, "button {index}: {pos:?}");
        }
        for (index, closed_zone) in closed.iter().enumerate() {
            app.toolbar[index].show();
            let open = runtime.toolbar_zones(&app, 1920.0, 1080.0)[index].expect("zone");
            let closed_zone = closed_zone.expect("zone");
            assert!(
                open.1.x * open.1.y > closed_zone.1.x * closed_zone.1.y,
                "panel {index} must enlarge the zone: {closed_zone:?} -> {open:?}"
            );
            app.toolbar[index] = crate::app::HoverPanel::default();
        }
    }

    #[test]
    fn status_bars_and_view_checkmarks_follow_the_settings() {
        let Some((runtime, mut app, atlas)) = default_fixture() else {
            return;
        };
        app.select_first();
        let is_bar_text = |text: &str| {
            let mut parts = text.split(" / ");
            matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None)
                if a.parse::<f32>().is_ok() && b.parse::<f32>().is_ok())
        };
        let bar_texts = |frame: &UiFrame| {
            frame
                .draws
                .iter()
                .filter(|draw| matches!(draw, UiDraw::Text { text, .. } if is_bar_text(text)))
                .count()
        };
        // The game's default: no status bars over the selected bibite.
        app.show_status_bars = false;
        let hidden = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert_eq!(bar_texts(&hidden), 0);
        app.show_status_bars = true;
        let shown = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert_eq!(bar_texts(&shown), 2, "health and energy");
        // A selected bibite without a chosen panel still shows the four tab buttons.
        assert!(!app.inspector_visible());
        assert!(
            shown
                .actions
                .iter()
                .filter(|(_, action)| matches!(action, UiAction::SetTab(_)))
                .count()
                >= 4
        );
    }

    #[test]
    fn the_info_tab_hides_while_the_information_panel_is_open() {
        let Some((runtime, mut app, atlas)) = default_fixture() else {
            return;
        };
        let open_buttons = |frame: &UiFrame| {
            frame
                .actions
                .iter()
                .filter(|(_, action)| *action == UiAction::Invoke(ControlCommand::OpenInformation))
                .count()
        };
        // `QuickStatsHolder/OpenButtons` is active until `InformationPanel.OpenPanel` (onOpen) turns it off.
        let closed = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert_eq!(open_buttons(&closed), 1, "the Info tab is there");
        app.panels.open(crate::ui::PanelKind::Information);
        let open = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert_eq!(open_buttons(&open), 0, "and gone while the panel is open");
        assert!(open.draws.len() < closed.draws.len());
        app.panels.close(crate::ui::PanelKind::Information);
        let again = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert_eq!(open_buttons(&again), 1, "`onClose` brings it back");
    }

    /// The captured Stats panel, its nodes by path from `StatsPanel`, and the fonts the layout measures with.
    fn stats_fixture(
        runtime: &RuntimeUi,
    ) -> Option<(
        &Node,
        std::collections::HashMap<String, &Node>,
        crate::text_sdf::SdfFonts,
    )> {
        fn find(node: &Node) -> Option<&Node> {
            if node.path.ends_with("/BibitePanels/StatsPanel") {
                return Some(node);
            }
            node.children.iter().find_map(find)
        }
        fn collect<'a>(node: &'a Node, into: &mut std::collections::HashMap<String, &'a Node>) {
            if let Some(rel) = crate::stats_states::StatsLayout::rel_of(&node.path) {
                into.insert(rel.to_owned(), node);
            }
            for child in &node.children {
                collect(child, into);
            }
        }
        let session = runtime.inspector_session.as_ref()?;
        let panel = session.canvases.iter().find_map(find)?;
        let mut captured = std::collections::HashMap::new();
        collect(panel, &mut captured);
        let fonts = crate::text_sdf::SdfFonts::discover(std::path::Path::new("."))?;
        Some((panel, captured, fonts))
    }

    #[test]
    fn the_stats_layout_solver_reproduces_the_captured_inspector() {
        let Some((runtime, _app, _atlas)) = default_fixture() else {
            return;
        };
        let Some((_, captured, fonts)) = stats_fixture(&runtime) else {
            return;
        };
        let solved = crate::unity_layout::solve(
            &crate::stats_layout_data::STATS_PANEL,
            &crate::unity_layout::Request {
                active: &|path: &str, _| captured.get(path).is_some_and(|node| node.active),
                text: &|path: &str, _| {
                    captured
                        .get(path)
                        .and_then(|node| node.texts.first())
                        .map(|text| text.text.clone())
                },
                elem: None,
            },
            &crate::stats_states::measure_for_tests(&fonts),
        );
        let mut compared = 0;
        let mut worst = 0.0_f32;
        let mut off = Vec::new();
        for (path, rect) in &solved.rects {
            let Some(node) = captured.get(path.as_str()) else {
                continue;
            };
            let rt = &node.rect;
            // Only the nodes a group places carry the layout (the rest follow their anchors).
            if !solved.placed.contains(path) {
                continue;
            }
            let parent = path
                .rsplit_once('/')
                .and_then(|(parent, _)| solved.rects.get(parent));
            let local = parent.map_or([rect[0], rect[1]], |parent| {
                [rect[0] - parent[0], rect[1] - parent[1]]
            });
            let ap = (
                local[0] + rt.pivot.x * rect[2],
                -(local[1] + (1.0 - rt.pivot.y) * rect[3]),
            );
            let deviation = (rect[2] - rt.size_delta.x)
                .abs()
                .max((rect[3] - rt.size_delta.y).abs())
                .max((ap.0 - rt.anchored_position.x).abs())
                .max((ap.1 - rt.anchored_position.y).abs());
            compared += 1;
            worst = worst.max(deviation);
            if deviation > 0.6 {
                off.push(path.clone());
            }
        }
        assert!(compared > 60, "only {compared} nodes compared");
        // Everything matches to within a pixel, the tag's title included: Roboto-Regular has no pair table (no "Ta" kerning) and TMP's
        // synthetic bold advances every character but the last 0.07 em more, in the Liberation titles ("Total Damages Suffered:") too.
        assert!(off.is_empty(), "off: {off:?}");
        assert!(worst < 0.6, "worst {worst}");
        // The panel itself.
        let panel = solved.rects["StatsPanel"];
        let node = captured["StatsPanel"];
        assert!((panel[2] - node.rect.size_delta.x).abs() < 1.0, "{panel:?}");
        assert!((panel[3] - node.rect.size_delta.y).abs() < 1.0, "{panel:?}");
    }

    #[test]
    fn the_stats_panel_follows_the_state_of_its_target() {
        use crate::stats_states::{StatsLayout, StatsState, StatsTexts};
        let Some((runtime, _app, _atlas)) = default_fixture() else {
            return;
        };
        let Some((panel, captured, fonts)) = stats_fixture(&runtime) else {
            return;
        };
        let texts = StatsTexts {
            species: captured["StatsPanel/MainSection/SpeciesHolder/SpeciesText"].texts[0]
                .text
                .clone(),
            tag: captured["StatsPanel/MainSection/TagHolder/Label Holder/TagLabel"].texts[0]
                .text
                .clone(),
        };
        let layout = |state: StatsState| StatsLayout::new(panel, &state, &texts, Some(&fonts));
        let rect = |layout: &StatsLayout, rel: &str| {
            layout
                .rect_of(rel, &captured[rel].rect)
                .unwrap_or(captured[rel].rect)
        };
        let close = |a: f32, b: f32| (a - b).abs() < 0.05;

        // The captured state is the captured panel.
        let base = layout(StatsState::default());
        for (rel, node) in &captured {
            let moved = rect(&base, rel);
            assert!(
                close(moved.anchored_position.x, node.rect.anchored_position.x)
                    && close(moved.anchored_position.y, node.rect.anchored_position.y)
                    && close(moved.size_delta.x, node.rect.size_delta.x)
                    && close(moved.size_delta.y, node.rect.size_delta.y),
                "{rel} moved in the captured state"
            );
            // The state decides the visibility of the nodes it knows; the rest is the capture's.
            if let Some(shows) = base.shows(rel) {
                assert_eq!(shows, node.active_in_hierarchy, "{rel}");
            }
        }

        // A first parent: the line (24 + 5 spacing) pushes the bibite's rows down.
        let parent = layout(StatsState {
            parent1: true,
            ..StatsState::default()
        });
        assert_eq!(
            parent.shows("StatsPanel/MainSection/Parents Line"),
            Some(true)
        );
        assert_eq!(
            parent.shows("StatsPanel/MainSection/Parents Line/parent1Button"),
            Some(true)
        );
        assert_eq!(
            parent.shows("StatsPanel/MainSection/Parents Line/parent2Button"),
            Some(false)
        );
        let stats = "StatsPanel/MainSection/BibiteStats";
        assert!(close(
            rect(&parent, stats).anchored_position.y,
            captured[stats].rect.anchored_position.y - 29.0
        ));
        let root = "StatsPanel";
        assert!(close(
            rect(&parent, root).size_delta.y,
            captured[root].rect.size_delta.y + 29.0
        ));
        // The line is where the prefab has it: under the tag row, 5 below it.
        let line = rect(&parent, "StatsPanel/MainSection/Parents Line");
        assert!(close(line.size_delta.y, 24.0));
        assert!(
            close(line.anchored_position.y, -82.0),
            "{:?}",
            line.anchored_position
        );
        // A second parent adds its button.
        let both = layout(StatsState {
            parent1: true,
            parent2: true,
            ..StatsState::default()
        });
        assert_eq!(
            both.shows("StatsPanel/MainSection/Parents Line/parent2Button"),
            Some(true)
        );
        let first = rect(&both, "StatsPanel/MainSection/Parents Line/parent1Button");
        let second = rect(&both, "StatsPanel/MainSection/Parents Line/parent2Button");
        assert!(second.anchored_position.x > first.anchored_position.x);

        // A corpse: the traits row (42 + 2) with the DEAD badge, Revive instead of Kill, no Lay and no Edit.
        let dead = layout(StatsState {
            dead: true,
            ..StatsState::default()
        });
        assert_eq!(
            dead.shows("StatsPanel/MainSection/BibiteStats/trait:"),
            Some(true)
        );
        assert_eq!(
            dead.shows("StatsPanel/MainSection/BibiteStats/trait:/Is DEAD"),
            Some(true)
        );
        assert_eq!(
            dead.shows("StatsPanel/MainSection/BibiteStats/trait:/Is RAD"),
            Some(false)
        );
        let generation = "StatsPanel/MainSection/BibiteStats/Generation";
        assert!(close(
            rect(&dead, generation).anchored_position.y,
            captured[generation].rect.anchored_position.y - 44.0
        ));
        assert_eq!(
            dead.shows("StatsPanel/ButtonHolder/KillBibiteButton"),
            Some(false)
        );
        assert_eq!(
            dead.shows("StatsPanel/ButtonHolder/LayBibiteEggButton"),
            Some(false)
        );
        assert_eq!(dead.shows("StatsPanel/ButtonHolder/Edit"), Some(false));
        assert_eq!(
            dead.shows("StatsPanel/ButtonHolder/ReviveCorpseButton"),
            Some(true)
        );
        assert_eq!(dead.shows("StatsPanel/ButtonHolder/Save"), Some(true));
        // Revive takes the first place of the column, Save the second.
        let revive = rect(&dead, "StatsPanel/ButtonHolder/ReviveCorpseButton");
        let save = rect(&dead, "StatsPanel/ButtonHolder/Save");
        assert!(close(
            revive.anchored_position.y,
            captured["StatsPanel/ButtonHolder/KillBibiteButton"]
                .rect
                .anchored_position
                .y
        ));
        assert!(
            close(save.anchored_position.y - revive.anchored_position.y, -90.0),
            "{save:?} {revive:?}"
        );
        // In a challenge a corpse cannot be revived either.
        let challenge = layout(StatsState {
            dead: true,
            challenge: true,
            ..StatsState::default()
        });
        assert_eq!(
            challenge.shows("StatsPanel/ButtonHolder/ReviveCorpseButton"),
            Some(false)
        );

        // An egg: its own section in place of the bibite's, no Lay; the panel is as wide as the prefab says (486.23).
        let egg = layout(StatsState {
            egg: true,
            ..StatsState::default()
        });
        assert_eq!(egg.shows("StatsPanel/MainSection/BibiteStats"), Some(false));
        assert_eq!(
            egg.shows("StatsPanel/MainSection/BibiteStats/Generation"),
            Some(false)
        );
        assert_eq!(egg.shows("StatsPanel/MainSection/EggStats"), Some(true));
        assert_eq!(
            egg.shows("StatsPanel/MainSection/EggStats/Hatch progress/ValueSlider"),
            Some(true)
        );
        assert_eq!(
            egg.shows("StatsPanel/ButtonHolder/LayBibiteEggButton"),
            Some(false)
        );
        assert_eq!(
            egg.shows("StatsPanel/ButtonHolder/KillBibiteButton"),
            Some(true)
        );
        let width = rect(&egg, "StatsPanel").size_delta.x;
        assert!((width - 486.23).abs() < 2.5, "{width}");
        let section = rect(&egg, "StatsPanel/MainSection/EggStats");
        // It starts where the bibite's rows did (70 below the top of the section).
        assert!(
            close(
                section.anchored_position.y,
                -(70.0 + section.size_delta.y / 2.0)
            ),
            "{section:?}"
        );

        // A long species name widens the panel (`SpeciesText` is as wide as its text).
        let long = StatsLayout::new(
            panel,
            &StatsState::default(),
            &StatsTexts {
                species: "Zzzzzzzzzz zzzzzzzzzzzzzzzzzzzz".to_owned(),
                ..texts.clone()
            },
            Some(&fonts),
        );
        assert!(
            rect(&long, "StatsPanel").size_delta.x
                > captured["StatsPanel"].rect.size_delta.x + 20.0
        );
    }

    /// The actions of the commands in a frame.
    fn commands(frame: &UiFrame) -> Vec<ControlCommand> {
        frame
            .actions
            .iter()
            .filter_map(|(_, action)| match action {
                UiAction::Invoke(command) => Some(*command),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_bibite_with_a_parent_shows_the_parents_line_and_its_button_selects_the_parent() {
        let Some((runtime, mut app, atlas)) = fixture("testdata/real/saves/lol.zip") else {
            return;
        };
        // A living bibite of the loaded save whose first parent is still there.
        let (child, parent) = app
            .world
            .bibites
            .iter()
            .filter(|(_, record)| !record.body.dead)
            .find_map(|(id, record)| {
                let parent = record.meta.parent1?;
                app.world.bibites.get(parent).map(|_| (id, parent))
            })
            .expect("a bibite with a parent in the lol save");
        app.select_bibite(child);
        app.open_bibite_panel(InspectorTab::Stats, false);
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        let shown = commands(&frame);
        assert!(
            shown.contains(&ControlCommand::SelectParent(true)),
            "the first parent's button"
        );
        assert!(
            !shown.contains(&ControlCommand::SelectParent(false)),
            "no second parent"
        );
        // Without a parent the line is not there.
        let orphan = app
            .world
            .bibites
            .iter()
            .find(|(_, record)| !record.body.dead && record.meta.parent1.is_none())
            .map(|(id, _)| id);
        if let Some(orphan) = orphan {
            app.select_bibite(orphan);
            let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
            assert!(!commands(&frame).contains(&ControlCommand::SelectParent(true)));
            app.select_bibite(child);
        }
        // The button selects the parent (`UserControl.SelectTarget(genes.parent1)`).
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SelectParent(true))]);
        assert_eq!(app.selected, Some(parent));
        // A parent that is gone leaves nothing to select.
        let mut app = app;
        let grandchild = child;
        app.select_bibite(grandchild);
        assert!(app.world.bibites.remove(parent).is_some());
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SelectParent(true))]);
        assert_eq!(app.selected, Some(grandchild));
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert!(!commands(&frame).contains(&ControlCommand::SelectParent(true)));
    }

    #[test]
    fn a_corpse_gets_revive_and_loses_kill_lay_and_edit() {
        let Some((runtime, mut app, atlas)) = fixture("testdata/real/saves/lol.zip") else {
            return;
        };
        let id = app
            .world
            .bibites
            .iter()
            .find(|(_, record)| !record.body.dead)
            .map(|(id, _)| id)
            .expect("a living bibite");
        app.select_bibite(id);
        app.open_bibite_panel(InspectorTab::Stats, false);
        let living = commands(&runtime.build_frame(&app, &atlas, 1920.0, 1080.0));
        for command in [
            ControlCommand::KillSelected,
            ControlCommand::LayEggs,
            ControlCommand::EditSelected,
            ControlCommand::SaveSelected,
        ] {
            assert!(living.contains(&command), "{command:?} for a living bibite");
        }
        assert!(!living.contains(&ControlCommand::ReviveSelected));
        // It dies: Revive instead of Kill, Lay and Edit; Save stays.
        app.world.bibites.get_mut(id).expect("record").body.dead = true;
        app.select_bibite(id);
        let dead = commands(&runtime.build_frame(&app, &atlas, 1920.0, 1080.0));
        assert!(dead.contains(&ControlCommand::ReviveSelected), "{dead:?}");
        assert!(dead.contains(&ControlCommand::SaveSelected));
        for command in [
            ControlCommand::KillSelected,
            ControlCommand::LayEggs,
            ControlCommand::EditSelected,
        ] {
            assert!(!dead.contains(&command), "{command:?} for a corpse");
        }
    }

    /// What the window composes its HUD from: the native capture with its HUD sessions adopted (`main.rs`) and the native sprite
    /// names. The sprite manifest is found from the working directory, so this skips when the tests do not run from the workspace.
    fn window_fixture() -> Option<(RuntimeUi, App, UiAssets)> {
        let (mut runtime, app, mut atlas) = native_fixture()?;
        runtime.adopt_native_fixed_sessions();
        if crate::load_native_sprite_manifest(&mut atlas).unwrap_or(0) == 0 {
            eprintln!("skipping: the native sprite manifest is not under the working directory");
            return None;
        }
        Some((runtime, app, atlas))
    }

    /// The texts a frame draws.
    fn drawn_texts(frame: &UiFrame) -> Vec<String> {
        frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// The hit rectangle `(pos, size)` of the first control with `action`.
    fn hit_of(frame: &UiFrame, action: UiAction) -> Option<(Vec2, Vec2)> {
        let (id, _) = frame.actions.iter().find(|(_, a)| *a == action)?;
        frame
            .hits
            .iter()
            .find(|(hit, ..)| hit == id)
            .map(|(_, pos, size)| (*pos, *size))
    }

    /// An egg of `app`'s world (laid by a living bibite), selected, with `tab` open and the numbers of its Stats panel set to known
    /// values: 8 s to hatch of which 4 are gone, 12.34 E stored of which 4 x 4 / 8 = 2 are committed to the body, a 0.6 size gene.
    fn selected_egg(app: &mut App, tab: InspectorTab) -> bibites_core::ids::EggId {
        let parent = app
            .world
            .live_bibite_ids()
            .into_iter()
            .next()
            .expect("a living bibite");
        assert!(app.world.lay_egg_once(parent) >= 1);
        let egg = app.world.eggs.ids().last().expect("an egg");
        {
            let record = app.world.eggs.get_mut(egg).expect("the egg");
            record.hatch.hatch_time = 8.0;
            record.hatch.hatch_progress = 4.0;
            record.hatch.total_required_energy = 4.0;
            record.hatch.energy = 12.34;
            record.size_gene = 0.6;
        }
        app.select_egg(egg);
        app.open_bibite_panel(tab, false);
        app.refresh_overlay(0.0);
        egg
    }

    #[test]
    fn an_eggs_stats_panel_shows_the_egg_section_in_the_games_formats() {
        let Some((runtime, mut app, atlas)) = window_fixture() else {
            return;
        };
        let egg = selected_egg(&mut app, InspectorTab::Stats);
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        let texts = drawn_texts(&frame);
        // The egg's rows (`FloatValueTextHandle` formats read from the prefab) and not the bibite's.
        for expected in [
            "Egg State:",
            "Total energy:",
            "Hatch progress:",
            "Energy Stored:",
        ] {
            assert!(
                texts.contains(&expected.to_owned()),
                "{expected}: {texts:?}"
            );
        }
        for absent in ["Time Alive:", "Alive Children:", "Distance Traveled:"] {
            assert!(!texts.contains(&absent.to_owned()), "{absent} of a bibite");
        }
        let view = app.world.egg_stats(egg).expect("the egg");
        assert!((view.egg_area - 1.8).abs() < 1e-5, "{}", view.egg_area);
        // 5 x 0.6^2 = 1.8 u2 (decade 0: no prefix); 12.34 E; 4.0 s of 8.0 s; 12.34 - 4 x 4 / 8 = 10.34 E of 12.34 E.
        for expected in [
            "1.80 u\u{b2}".to_owned(),
            crate::stats_values::MASS.format(view.egg_mass),
            "12.34 E".to_owned(),
            "4.0s".to_owned(),
            "8.0s".to_owned(),
            "10.3 E".to_owned(),
            "12.3 E".to_owned(),
        ] {
            assert!(texts.contains(&expected), "{expected:?} in {texts:?}");
        }
        // The generation row of the egg section (the bibite's is hidden).
        assert!(
            texts.contains(&format!("{}", view.generation)),
            "the generation"
        );
        // The bars: a cap of 16 plus the share of the other 184 (200 x 26 bars, `targetppu` 0.5).
        let fills: Vec<(Rgba, f32)> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::NineSlice { tint, size, .. }
                    if *tint == [255, 52, 52, 255] || *tint == [255, 199, 0, 255] =>
                {
                    Some((*tint, size.x))
                }
                _ => None,
            })
            .collect();
        let hatch = fills
            .iter()
            .find(|(tint, _)| *tint == [255, 52, 52, 255])
            .expect("the hatch progress fill");
        assert!((hatch.1 - (16.0 + 0.5 * 184.0)).abs() < 0.1, "{hatch:?}");
        let stored = fills
            .iter()
            .find(|(tint, _)| *tint == [255, 199, 0, 255])
            .expect("the stored energy fill");
        assert!(
            (stored.1 - (16.0 + 10.34 / 12.34 * 184.0)).abs() < 0.1,
            "{stored:?}"
        );
        // Kill, Save and Edit are the egg's buttons; Lay and Revive are not.
        let shown = commands(&frame);
        assert!(shown.contains(&ControlCommand::KillSelected));
        assert!(shown.contains(&ControlCommand::SaveSelected));
        assert!(!shown.contains(&ControlCommand::LayEggs));
        assert!(!shown.contains(&ControlCommand::ReviveSelected));
    }

    #[test]
    fn the_hatch_bar_fills_up_as_the_egg_incubates() {
        let Some((runtime, mut app, atlas)) = window_fixture() else {
            return;
        };
        let egg = selected_egg(&mut app, InspectorTab::Stats);
        let fill = |app: &App| -> f32 {
            runtime
                .build_frame(app, &atlas, 1920.0, 1080.0)
                .draws
                .iter()
                .find_map(|draw| match draw {
                    UiDraw::NineSlice { tint, size, .. } if *tint == [255, 52, 52, 255] => {
                        Some(size.x)
                    }
                    _ => None,
                })
                .expect("the hatch progress fill")
        };
        let half = fill(&app);
        app.world
            .eggs
            .get_mut(egg)
            .expect("the egg")
            .hatch
            .hatch_progress = 8.0;
        app.refresh_overlay(0.0);
        assert!((fill(&app) - 200.0).abs() < 0.1, "a full bar is 200 wide");
        app.world
            .eggs
            .get_mut(egg)
            .expect("the egg")
            .hatch
            .hatch_progress = 0.0;
        app.refresh_overlay(0.0);
        assert!((fill(&app) - 16.0).abs() < 0.1, "an empty bar is its cap");
        assert!(half > 16.0 && half < 200.0);
    }

    #[test]
    fn an_egg_has_no_biology_button_and_its_brain_button_takes_its_place() {
        let Some((runtime, mut app, atlas)) = window_fixture() else {
            return;
        };
        // A bibite has the four tab buttons, 60 wide, 70 apart.
        let id = app
            .world
            .live_bibite_ids()
            .into_iter()
            .next()
            .expect("a living bibite");
        app.select_bibite(id);
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        let (biology, _) = hit_of(&frame, UiAction::SetTab(InspectorTab::Biology))
            .expect("the Biology button of a bibite");
        let (brain, size) = hit_of(&frame, UiAction::SetTab(InspectorTab::Brain))
            .expect("the Brain button of a bibite");
        assert!((brain.x - biology.x - 70.0).abs() < 0.5, "{brain:?}");
        assert!((size.x - 60.0).abs() < 0.5);
        // An egg has no Biology button; the Brain button sits where Biology was.
        selected_egg(&mut app, InspectorTab::Stats);
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        assert!(hit_of(&frame, UiAction::SetTab(InspectorTab::Biology)).is_none());
        let (moved, _) = hit_of(&frame, UiAction::SetTab(InspectorTab::Brain))
            .expect("the Brain button of an egg");
        assert!((moved.x - biology.x).abs() < 0.5, "{moved:?} {biology:?}");
        assert!(hit_of(&frame, UiAction::SetTab(InspectorTab::Genes)).is_some());
        assert!(hit_of(&frame, UiAction::SetTab(InspectorTab::Stats)).is_some());
    }

    #[test]
    fn an_eggs_genes_and_brain_tabs_show_its_own_genes_and_brain() {
        let Some((runtime, mut app, atlas)) = window_fixture() else {
            return;
        };
        let egg = selected_egg(&mut app, InspectorTab::Genes);
        // Make the egg's genes recognisable: a value of the first gene row (`LayTime`) no bibite of the world has.
        let genome = &mut app.world.eggs.get_mut(egg).expect("the egg").genome;
        genome.set_gene(bibites_core::genome::Gene::LayTime, 12.3456);
        let lay_time = format!("{:.3}", genome.gene(bibites_core::genome::Gene::LayTime));
        app.refresh_overlay(0.0);
        let texts = drawn_texts(&runtime.build_frame(&app, &atlas, 1920.0, 1080.0));
        assert!(
            texts.contains(&lay_time),
            "the egg's LayTime {lay_time} in the Genes panel: {texts:?}"
        );
        let record = app.world.eggs.get(egg).expect("the egg");
        let species = record
            .meta
            .species
            .and_then(|species| app.world.species.get(species))
            .map(|species| species.name())
            .expect("the egg's species");
        assert!(texts.contains(&species), "{species} in {texts:?}");
        // The Brain tab: the egg's own node and connection counts and its diagram.
        let nodes = record.brain.node_count();
        let synapses = record.brain.synapse_count();
        app.open_bibite_panel(InspectorTab::Brain, false);
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        let texts = drawn_texts(&frame);
        assert!(texts.contains(&nodes.to_string()), "{nodes} in {texts:?}");
        assert!(texts.contains(&synapses.to_string()), "{synapses}");
        let neurons = frame
            .draws
            .iter()
            .filter(|draw| matches!(draw, UiDraw::Image { .. }))
            .count();
        assert!(neurons > 10, "the egg's brain is drawn ({neurons} images)");
    }

    /// A living bibite of the fixture selected with the inspector's Brain tab open.
    fn selected_bibite_with_brain_tab(app: &mut App) -> bibites_core::ids::BibiteId {
        let id = app
            .world
            .live_bibite_ids()
            .into_iter()
            .next()
            .expect("a living bibite");
        app.select_bibite(id);
        app.open_bibite_panel(InspectorTab::Brain, false);
        app.refresh_overlay(0.0);
        id
    }

    #[test]
    fn the_inspectors_brain_tab_draws_the_laid_out_diagram_and_its_controls_work() {
        let Some((runtime, mut app, atlas)) = window_fixture() else {
            return;
        };
        selected_bibite_with_brain_tab(&mut app);
        let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
        let state = app.inspector_brain.as_ref().expect("the tab was filled");
        // A control per node the diagram draws (not the 13 neurons of the capture), each where the layout puts it.
        let nodes: Vec<u16> = frame
            .actions
            .iter()
            .filter_map(|(_, action)| match action {
                UiAction::InspectorBrainNode(node) => Some(*node),
                _ => None,
            })
            .collect();
        assert_eq!(nodes.len(), state.diagram.used.len(), "{nodes:?}");
        let geometry = crate::inspector_brain::Geometry::of(&state.diagram);
        let holder = geometry.holder();
        let &first = state.diagram.used.first().expect("a node");
        let at = state.diagram.position(first).expect("a place");
        let (pos, size) = hit_of(&frame, UiAction::InspectorBrainNode(first as u16))
            .expect("the first node's control");
        assert!(
            (pos.x + size.x / 2.0 - (holder[0] + at[0])).abs() < 0.5
                && (pos.y + size.y / 2.0 - (holder[1] - at[1])).abs() < 0.5,
            "{pos:?} {size:?} {holder:?} {at:?}"
        );
        // A diagram of the captured size leaves the panel as captured: the Sort button where the capture has it
        // (415, 513 to 543 from the bottom, 80 x 30).
        if (geometry.width, geometry.height) == (580.0, 300.0) {
            let (pos, size) = hit_of(&frame, UiAction::InspectorBrainSort).expect("Sort");
            assert!(
                (pos.x - 415.0).abs() < 0.5
                    && (pos.y - (1080.0 - 543.0)).abs() < 0.5
                    && (size.x - 80.0).abs() < 0.5
                    && (size.y - 30.0).abs() < 0.5,
                "{pos:?} {size:?}"
            );
        }
        // The Sort button and the toggle are controls of the panel, under the holder.
        for action in [
            UiAction::InspectorBrainSort,
            UiAction::BrainToggleDifferences,
        ] {
            let (pos, _) = hit_of(&frame, action).unwrap_or_else(|| panic!("{action:?}"));
            assert!(pos.y > holder[1] + holder[3], "{action:?} under the holder");
        }
        // The info row counts what the diagram draws.
        let texts = drawn_texts(&frame);
        assert!(
            texts.contains(&state.diagram.used.len().to_string()),
            "{texts:?}"
        );
        assert!(texts.contains(&state.diagram.graph.synapses.len().to_string()));
        assert!(texts.contains(&"Highlight differences".to_owned()));
        // The check mark of the toggle shows with the setting.
        let checkmarks = |frame: &UiFrame| -> usize {
            frame
                .draws
                .iter()
                .filter(|draw| {
                    matches!(draw, UiDraw::Image { size, .. } if (size.x - 25.0).abs() < 0.5 && (size.y - 25.0).abs() < 0.5)
                })
                .count()
        };
        let off = checkmarks(&frame);
        app.apply_ui_actions(&[UiAction::BrainToggleDifferences]);
        let on = checkmarks(&runtime.build_frame(&app, &atlas, 1920.0, 1080.0));
        assert_eq!(on, off + 1, "the check mark appears");
    }

    #[test]
    fn the_brain_panel_grows_with_its_diagram_and_moves_the_bottom_line_down() {
        use bibites_core::brain::NodeType;
        let Some((runtime, mut app, atlas)) = window_fixture() else {
            return;
        };
        let id = selected_bibite_with_brain_tab(&mut app);
        let measure = |app: &App| -> (f32, f32, f32) {
            let frame = runtime.build_frame(app, &atlas, 1920.0, 1080.0);
            let state = app.inspector_brain.as_ref().expect("filled");
            let (sort, _) = hit_of(&frame, UiAction::InspectorBrainSort).expect("Sort");
            let panel_bottom = frame
                .draws
                .iter()
                .filter_map(|draw| match draw {
                    UiDraw::NineSlice {
                        pos, size, tint, ..
                    } if tint[3] == 225 => Some(pos.y + size.y),
                    _ => None,
                })
                .fold(0.0_f32, f32::max);
            (
                crate::inspector_brain::Geometry::of(&state.diagram).height,
                sort.y,
                panel_bottom,
            )
        };
        let (height, sort, bottom) = measure(&app);
        // Many more nodes in the brain: more groups, a taller diagram.
        {
            let brain = &mut app.world.bibites.get_mut(id).expect("record").brain;
            for index in 0..14 {
                let node = brain
                    .add_hidden_node(NodeType::Sigmoid, None)
                    .expect("a hidden node");
                assert!(
                    brain
                        .add_synapse_connection(index, node, 1.0, true)
                        .is_some()
                );
                assert!(
                    brain
                        .add_synapse_connection(
                            node,
                            bibites_core::brain::FIRST_OUTPUT + index % 10,
                            1.0,
                            true
                        )
                        .is_some()
                );
            }
        }
        app.inspector_brain = None;
        app.refresh_overlay(0.0);
        let (taller, sort2, bottom2) = measure(&app);
        assert!(taller > height, "{taller} against {height}");
        // The Sort button and the panel's lower edge went down by as much as the holder grew.
        assert!(
            ((sort2 - sort) - (taller - height)).abs() < 0.5,
            "sort {sort} -> {sort2}, holder {height} -> {taller}"
        );
        assert!(
            ((bottom2 - bottom) - (taller - height)).abs() < 0.5,
            "panel bottom {bottom} -> {bottom2}"
        );
    }

    #[test]
    fn a_long_species_name_widens_the_brain_panels_species_row_and_moves_the_distance_and_the_help_button()
     {
        let Some((runtime, mut app, atlas)) = window_fixture() else {
            return;
        };
        // The starter bibite of the fixture belongs to a species: rename it, short then long.
        let id = selected_bibite_with_brain_tab(&mut app);
        let species = app
            .world
            .bibites
            .get(id)
            .and_then(|record| record.meta.species)
            .expect("a species");
        // The distance reads "+N%" (`FloatInfoText` between the parentheses); "Help" is the button's label.
        let at = |app: &App, text: &str| -> Option<Vec2> {
            runtime
                .build_frame(app, &atlas, 1920.0, 1080.0)
                .draws
                .iter()
                .find_map(|draw| match draw {
                    UiDraw::Text {
                        text: shown, pos, ..
                    } if shown == text || (text == "%" && shown.ends_with('%')) => Some(*pos),
                    _ => None,
                })
        };
        assert!(app.world.species.rename(species, "Aa bb"));
        app.refresh_overlay(0.0);
        let short_distance = at(&app, "%").expect("the distance text");
        let short_help = at(&app, "Help").expect("the help button");
        assert!(
            app.world
                .species
                .rename(species, "Zzzzzzzzzzzzz zzzzzzzzzzzzzzzzzzzzzz")
        );
        app.refresh_overlay(0.0);
        let long_distance = at(&app, "%").expect("the distance text");
        let long_help = at(&app, "Help").expect("the help button");
        // The name takes more room, so what follows it moves right; the first row stays as wide as the panel.
        assert!(
            long_distance.x > short_distance.x + 100.0,
            "{short_distance:?} -> {long_distance:?}"
        );
        // The captured name is wider than "Aa bb": the distance of the short name is to the left of the captured one.
        assert!(short_distance.x < 230.0, "{short_distance:?}");
        // The help button sits at the right end of the row: it only moves when the panel itself grows.
        assert!(long_help.x >= short_help.x);
    }

    #[test]
    fn the_status_bars_fill_with_the_targets_health_and_energy_and_scale_with_the_zoom() {
        let Some((runtime, mut app, atlas)) = window_fixture() else {
            return;
        };
        app.show_status_bars = true;
        let id = app
            .world
            .live_bibite_ids()
            .into_iter()
            .next()
            .expect("a living bibite");
        app.select_bibite(id);
        let health = atlas
            .texture_of_alias("health_filled")
            .expect("health_filled");
        let energy = atlas
            .texture_of_alias("energy_filled")
            .expect("energy_filled");
        // `(pos, size)` of a bar's filled part, and the part of the sprite it shows.
        let bars = |app: &App| -> (Option<(Vec2, Vec2, f32)>, Option<(Vec2, Vec2, f32)>) {
            let frame = runtime.build_frame(app, &atlas, 1920.0, 1080.0);
            let find = |slot: u16| {
                frame.draws.iter().find_map(|draw| match draw {
                    UiDraw::ImageRegion {
                        texture,
                        region,
                        pos,
                        size,
                        ..
                    } if *texture == slot => {
                        let full = atlas.texture(slot).expect("texture").width as f32;
                        Some((*pos, *size, region[2] / full))
                    }
                    _ => None,
                })
            };
            (find(health), find(energy))
        };
        // Full: the whole sprite (no part cut off: the game's fillAmount 1 draws it as is).
        {
            let record = app.world.bibites.get_mut(id).expect("record");
            record.body.health.amount = record.body.health.max_amount;
            record.body.energy.amount = record.body.energy.max_amount;
        }
        app.refresh_overlay(0.0);
        let (full_health, full_energy) = bars(&app);
        let (full_health, full_energy) = (
            full_health.expect("health bar"),
            full_energy.expect("energy bar"),
        );
        assert!((full_health.2 - 1.0).abs() < 1e-3 && (full_energy.2 - 1.0).abs() < 1e-3);
        // Half the health and a quarter of the energy: the left part of the sprite, as wide.
        {
            let record = app.world.bibites.get_mut(id).expect("record");
            record.body.health.amount = record.body.health.max_amount * 0.5;
            record.body.energy.amount = record.body.energy.max_amount * 0.25;
        }
        app.refresh_overlay(0.0);
        let (half_health, quarter_energy) = bars(&app);
        let (half_health, quarter_energy) = (
            half_health.expect("health bar"),
            quarter_energy.expect("energy bar"),
        );
        assert!((half_health.2 - 0.5).abs() < 1e-3, "{half_health:?}");
        assert!((quarter_energy.2 - 0.25).abs() < 1e-3, "{quarter_energy:?}");
        assert!((half_health.1.x - 0.5 * full_health.1.x).abs() < 0.01);
        assert!((quarter_energy.1.x - 0.25 * full_energy.1.x).abs() < 0.01);
        // The bars begin where they began (the left edge does not move with the fill).
        assert!((half_health.0.x - full_health.0.x).abs() < 0.01);
        // Zooming in doubles the bars (`scale = targetScale x 200 / camSize`) about the bar's pivot at the centre of the screen.
        let before = app.camera.zoom;
        app.camera.zoom = before * 2.0;
        let (zoomed, _) = bars(&app);
        let zoomed = zoomed.expect("health bar");
        assert!(
            (zoomed.1.x - 2.0 * half_health.1.x).abs() < 0.05,
            "{zoomed:?} {half_health:?}"
        );
        assert!((zoomed.1.y - 2.0 * half_health.1.y).abs() < 0.05);
        app.camera.zoom = before;
    }

    #[test]
    fn the_brain_tabs_buttons_and_neurons_work_through_the_windows_pointer_routing() {
        use crate::app::FrameInput;
        let Some((runtime, mut app, atlas)) = window_fixture() else {
            return;
        };
        // The window's size is the frame's: the pointer's pixels are the frame's.
        app.camera.screen = Vec2::new(1920.0, 1080.0);
        selected_bibite_with_brain_tab(&mut app);
        let route = |app: &mut App, input: FrameInput| {
            let frame = runtime.build_frame(app, &atlas, 1920.0, 1080.0);
            crate::route_ui_frame(app, &frame, input, Some(&runtime), 1920.0, 1080.0)
        };
        let centre_of = |app: &App, action: UiAction| -> Vec2 {
            let frame = runtime.build_frame(app, &atlas, 1920.0, 1080.0);
            let (pos, size) = hit_of(&frame, action).unwrap_or_else(|| panic!("{action:?}"));
            Vec2::new(pos.x + size.x / 2.0, pos.y + size.y / 2.0)
        };
        let serial = |app: &App| app.inspector_brain.as_ref().expect("filled").serial();
        // A click on Sort lays the diagram out again.
        let sort = centre_of(&app, UiAction::InspectorBrainSort);
        let before = serial(&app);
        route(
            &mut app,
            FrameInput {
                cursor: sort,
                primary_pressed: true,
                primary_down: true,
                ..FrameInput::default()
            },
        );
        let routed = route(
            &mut app,
            FrameInput {
                cursor: sort,
                primary_released: true,
                ..FrameInput::default()
            },
        );
        assert!(!routed.primary_released, "the UI took the click");
        assert_ne!(serial(&app), before, "Sort sorted");
        // A click on the toggle flips the setting.
        let toggle = centre_of(&app, UiAction::BrainToggleDifferences);
        assert!(!app.show_brain_differences);
        route(
            &mut app,
            FrameInput {
                cursor: toggle,
                primary_pressed: true,
                primary_down: true,
                ..FrameInput::default()
            },
        );
        route(
            &mut app,
            FrameInput {
                cursor: toggle,
                primary_released: true,
                ..FrameInput::default()
            },
        );
        assert!(app.show_brain_differences);
        // A neuron pressed follows the cursor while it is held and stays where it was let go.
        let node = *app
            .inspector_brain
            .as_ref()
            .expect("filled")
            .diagram
            .used
            .first()
            .expect("a node");
        let at = |app: &App| {
            app.inspector_brain
                .as_ref()
                .unwrap()
                .diagram
                .position(node)
                .unwrap()
        };
        let grab = centre_of(&app, UiAction::InspectorBrainNode(node as u16));
        let start = at(&app);
        let routed = route(
            &mut app,
            FrameInput {
                cursor: grab,
                primary_pressed: true,
                primary_down: true,
                ..FrameInput::default()
            },
        );
        assert!(!routed.primary_pressed && app.inspector_brain_drag.is_some());
        let routed = route(
            &mut app,
            FrameInput {
                cursor: grab + Vec2::new(30.0, 25.0),
                primary_down: true,
                ..FrameInput::default()
            },
        );
        assert!(
            !routed.primary_down,
            "the held neuron keeps the pointer from the world"
        );
        let moved = at(&app);
        assert!(
            (moved[0] - start[0] - 30.0).abs() < 1.0 && (moved[1] - start[1] + 25.0).abs() < 1.0,
            "{start:?} -> {moved:?}"
        );
        route(
            &mut app,
            FrameInput {
                cursor: grab + Vec2::new(30.0, 25.0),
                primary_released: true,
                ..FrameInput::default()
            },
        );
        assert!(app.inspector_brain_drag.is_none());
        assert_eq!(at(&app), moved);
    }

    #[test]
    fn the_brain_tabs_expanded_view_button_opens_the_expanded_view_through_the_windows_routing() {
        use crate::app::FrameInput;
        let Some((runtime, mut app, atlas)) = window_fixture() else {
            return;
        };
        app.camera.screen = Vec2::new(1920.0, 1080.0);
        selected_bibite_with_brain_tab(&mut app);
        let route = |app: &mut App, input: FrameInput| {
            let frame = runtime.build_frame(app, &atlas, 1920.0, 1080.0);
            crate::route_ui_frame(app, &frame, input, Some(&runtime), 1920.0, 1080.0)
        };
        // `ExpandedViewerButton`: `GUIManager.OpenBibitePanel(ExpendedBrainPanel)`.
        let button = {
            let frame = runtime.build_frame(&app, &atlas, 1920.0, 1080.0);
            let (pos, size) = hit_of(
                &frame,
                UiAction::Invoke(ControlCommand::OpenExpandedBrain),
            )
            .expect("the Expanded View button is a control of the Brain panel");
            Vec2::new(pos.x + size.x / 2.0, pos.y + size.y / 2.0)
        };
        assert!(app.expanded_brain.is_none());
        route(
            &mut app,
            FrameInput {
                cursor: button,
                primary_pressed: true,
                primary_down: true,
                ..FrameInput::default()
            },
        );
        let routed = route(
            &mut app,
            FrameInput {
                cursor: button,
                primary_released: true,
                ..FrameInput::default()
            },
        );
        assert!(!routed.primary_released, "the UI took the click");
        assert!(
            app.expanded_brain.is_some(),
            "the click opened the Expanded View"
        );
    }

    #[test]
    fn dynamic_row_ids_are_stable_and_distinct() {
        let first = dynamic_row_id("main_menu_load_game", "UI/LoadGamePanel/LoadList", 0);
        let second = dynamic_row_id("main_menu_load_game", "UI/LoadGamePanel/LoadList", 1);
        assert_ne!(first, second);
        assert_eq!(
            first,
            dynamic_row_id("main_menu_load_game", "UI/LoadGamePanel/LoadList", 0)
        );
    }

    #[test]
    fn settings_value_metadata_and_pointer_proposals() {
        let Some((runtime, _app, _atlas)) = save_fixture() else {
            return;
        };
        let controls = runtime.controls(1920.0, 1080.0);
        for session in [
            "main_menu_settings_graphics",
            "main_menu_settings_gameplay",
            "main_menu_settings_hotkeys",
        ] {
            let session_controls: Vec<_> = controls
                .iter()
                .filter(|control| control.session == session)
                .collect();
            assert!(
                session_controls.iter().any(|control| control
                    .value
                    .as_ref()
                    .is_some_and(|value| value.is_on.is_some())),
                "{session} has no captured toggle values"
            );
            let slider = session_controls
                .iter()
                .find(|control| {
                    control.kind == ControlKind::Slider
                        && control.value.as_ref().is_some_and(|value| {
                            value.value.is_some()
                                && value.min_value.is_some()
                                && value.max_value.is_some()
                        })
                })
                .unwrap_or_else(|| panic!("{session} has no captured slider value"));
            let meta = slider.value.as_ref().expect("slider meta");
            let min = meta.min_value.expect("min");
            let max = meta.max_value.expect("max");
            assert!(max > min);
            let left = Vec2::new(slider.rect[0], slider.rect[1] + slider.rect[3] * 0.5);
            let right = Vec2::new(
                slider.rect[0] + slider.rect[2],
                slider.rect[1] + slider.rect[3] * 0.5,
            );
            let left_value = slider.proposed_value(left).expect("left proposal");
            let right_value = slider.proposed_value(right).expect("right proposal");
            let extract = |action: UiAction| match action {
                UiAction::SetValue(ControlValue::Float(value)) => value,
                other => panic!("unexpected slider action {other:?}"),
            };
            assert!((extract(left_value) - min).abs() < 1e-3);
            assert!((extract(right_value) - max).abs() < 1e-3);

            let toggle = session_controls
                .iter()
                .find(|control| {
                    control.kind == ControlKind::Toggle
                        && control
                            .value
                            .as_ref()
                            .and_then(|value| value.is_on)
                            .is_some()
                })
                .unwrap_or_else(|| panic!("{session} has no captured toggle"));
            let current = toggle.value.as_ref().and_then(|value| value.is_on).unwrap();
            assert_eq!(
                toggle.proposed_value(Vec2::ZERO),
                Some(UiAction::SetValue(ControlValue::Flag(!current)))
            );

            let dropdown = session_controls
                .iter()
                .find(|control| {
                    control.kind == ControlKind::Dropdown
                        && control
                            .value
                            .as_ref()
                            .is_some_and(|value| !value.options.is_empty())
                })
                .unwrap_or_else(|| panic!("{session} has no captured dropdown options"));
            assert!(!dropdown.option_labels().is_empty());
            assert_eq!(
                dropdown.value.as_ref().expect("meta").option_rects.len(),
                dropdown.option_labels().len()
            );
            assert_eq!(
                dropdown.proposed_choice(0),
                Some(UiAction::SetValue(ControlValue::Choice(0)))
            );
            assert!(
                dropdown
                    .proposed_choice(dropdown.option_labels().len())
                    .is_none()
            );
        }
    }

    #[test]
    fn settings_pages_register_all_controls_with_values_and_hotkeys() {
        let Some((runtime, _app, _atlas)) = save_fixture() else {
            return;
        };
        let sessions = [
            "main_menu_settings",
            "main_menu_settings_graphics",
            "main_menu_settings_gameplay",
            "main_menu_settings_hotkeys",
        ];
        let selectors = [
            (
                "GraphicsButton",
                "UI/SettingsPanel/SelectorPanel/List/GraphicsButton",
            ),
            (
                "Gameplay Button",
                "UI/SettingsPanel/SelectorPanel/List/Gameplay Button",
            ),
            (
                "Controls Button",
                "UI/SettingsPanel/SelectorPanel/List/Controls Button",
            ),
            (
                "CloseButton",
                "UI/SettingsPanel/SelectorPanel/List/CloseButton",
            ),
        ];
        for session in sessions {
            let controls = runtime.session_controls(session, 1920.0, 1080.0);
            assert!(!controls.is_empty(), "{session} has no controls");
            let mut ids = std::collections::HashSet::new();
            for control in &controls {
                assert_eq!(
                    control.id,
                    control_id(session, &control.path),
                    "unstable id for {session}/{}",
                    control.path
                );
                assert!(ids.insert(control.id), "duplicate id in {session}");
            }
            for (name, path) in selectors {
                let selector = controls
                    .iter()
                    .find(|control| control.name == name && control.path == path)
                    .unwrap_or_else(|| panic!("{session} missing selector {name} at {path}"));
                assert!(selector.rect[2] > 0.0 && selector.rect[3] > 0.0);
            }

            let page_controls: Vec<_> = controls
                .iter()
                .filter(|control| control.path.starts_with("UI/SettingsPanel/PagesPanels/"))
                .collect();
            assert!(
                !page_controls.is_empty(),
                "{session} has no Settings-page controls"
            );
            for control in &page_controls {
                if matches!(
                    control.kind,
                    ControlKind::Toggle | ControlKind::Slider | ControlKind::Dropdown
                ) {
                    let value = control
                        .value
                        .as_ref()
                        .unwrap_or_else(|| panic!("missing value on {session}/{}", control.path));
                    match control.kind {
                        ControlKind::Toggle => assert!(value.is_on.is_some()),
                        ControlKind::Slider => {
                            assert!(value.value.is_some());
                            let min = value.min_value.expect("slider min");
                            let max = value.max_value.expect("slider max");
                            assert!(max >= min);
                        }
                        ControlKind::Dropdown => {
                            assert!(!value.options.is_empty());
                            assert_eq!(value.option_rects.len(), value.options.len());
                            assert_eq!(
                                control.proposed_choice(0),
                                Some(UiAction::SetValue(ControlValue::Choice(0)))
                            );
                        }
                        _ => {}
                    }
                }
            }

            let expected = controls.len();
            let frame = runtime
                .build_named_session_frame(session, &_atlas, 1920.0, 1080.0)
                .unwrap_or_else(|| panic!("missing frame for {session}"));
            assert!(
                frame.hits.len() >= expected,
                "hit coverage for {session}: {} vs {expected}",
                frame.hits.len()
            );
        }

        let blocks = runtime.hotkey_blocks("main_menu_settings_hotkeys");
        assert!(!blocks.is_empty(), "no captured hotkey blocks");
        assert!(
            blocks
                .iter()
                .all(|block| block.path.contains("/PagesPanels/Hotkeys/"))
        );
        let lines: Vec<&str> = blocks
            .iter()
            .flat_map(|block| block.lines.iter().map(String::as_str))
            .collect();
        assert!(lines.iter().any(|line| line.contains("Camera Controls")));
        assert!(lines.iter().any(|line| line.contains("Selection Hotkeys")));
    }

    #[test]
    fn library_and_newgame_controls_are_hit_testable_with_dropdown_values() {
        let Some((runtime, _app, _atlas)) = save_fixture() else {
            return;
        };
        let controls = runtime.controls(1920.0, 1080.0);
        for (session, names) in [
            (
                "library_selector_flow",
                &["CreateNew", "Templates", "Edit Button"][..],
            ),
            ("main_menu_bibite_library", &["CreateNew", "Templates"][..]),
        ] {
            for name in names {
                assert!(
                    controls
                        .iter()
                        .any(|control| control.session == session && control.name == *name),
                    "missing {session}/{name}"
                );
            }
            let expected = controls
                .iter()
                .filter(|control| control.session == session)
                .count();
            let expected_hits = expected;
            let frame = runtime
                .build_named_session_frame(session, &_atlas, 1920.0, 1080.0)
                .expect("library frame");
            assert!(
                frame.hits.len() >= expected_hits,
                "hit coverage for {session}: {} vs {}",
                frame.hits.len(),
                expected_hits
            );
        }

        let newgame = controls
            .iter()
            .filter(|control| control.session == "newgame_scenario_options")
            .find(|control| {
                control.kind == ControlKind::Dropdown
                    && control
                        .value
                        .as_ref()
                        .is_some_and(|value| !value.options.is_empty())
            })
            .expect("newgame dropdown");
        let meta = newgame.value.as_ref().expect("meta");
        assert!(meta.options.len() >= 5);
        assert!(meta.option_rects.len() == meta.options.len());
        assert_eq!(
            newgame.proposed_choice(1),
            Some(UiAction::SetValue(ControlValue::Choice(1)))
        );
    }

    #[test]
    fn functional_sessions_parse_have_rects_labels_and_known_bindings() {
        let functional = [
            "main_menu_functional_title",
            "main_menu_load_game",
            "main_menu_settings",
            "main_menu_settings_graphics",
            "main_menu_settings_gameplay",
            "main_menu_settings_hotkeys",
            "main_menu_new_game",
            "main_menu_bibite_library",
            "main_menu_bibite_editor_genes",
            "main_menu_bibite_editor_brain",
            "main_menu_achievements",
            "main_menu_additional_tools",
            "main_menu_credits",
            "main_menu_new_panel",
            "main_menu_workshop",
            "main_menu_scenario_editor",
            "main_world_toolbar",
            "main_world_tool_placer",
            "main_world_tool_user_actions",
            "main_world_tool_view",
            "main_world_dynamic_parameters",
            "main_world_lineage",
            "main_world_information",
            "main_world_settings_panel",
        ];
        let Some((runtime, _app, _atlas)) = save_fixture() else {
            return;
        };
        let names = runtime.session_names();
        for name in functional {
            assert!(
                names.iter().any(|candidate| candidate == name),
                "functional session {name} not parsed"
            );
        }
        let controls = runtime.controls(1920.0, 1080.0);
        for name in functional {
            let session_controls: Vec<_> = controls
                .iter()
                .filter(|control| control.session == name)
                .collect();
            assert!(
                !session_controls.is_empty(),
                "functional session {name} has no active controls"
            );
            for control in session_controls {
                assert!(
                    !control.label.is_empty(),
                    "empty label for {}",
                    control.path
                );
                assert!(control.rect.iter().all(|value| value.is_finite()));
            }
        }

        let find = |session: &str, name: &str| {
            controls
                .iter()
                .find(|control| control.session == session && control.name == name)
                .unwrap_or_else(|| panic!("missing {session}/{name}"))
        };
        assert_eq!(
            find("main_menu_functional_title", "Settings").action,
            Some(UiAction::OpenPanel(PanelKind::Settings))
        );
        assert_eq!(
            find("main_menu_functional_title", "Quit").action,
            Some(UiAction::Quit)
        );
        assert_eq!(
            find("main_world_toolbar", "PauseButton").action,
            Some(UiAction::TogglePause)
        );
        assert_eq!(
            find("main_world_toolbar", "SettingsButton").action,
            Some(UiAction::Invoke(ControlCommand::OpenEscapeMenu))
        );
        assert_eq!(
            find("main_world_tool_placer", "PlacePellets").action,
            Some(UiAction::Invoke(ControlCommand::SelectTool(
                ToolKind::Pellet
            )))
        );
        assert_eq!(
            find("main_world_lineage", "AncestralLineageButton").action,
            Some(UiAction::OpenPanel(PanelKind::Lineage))
        );
        assert_eq!(
            find("main_menu_load_game", "LoadButton").action,
            Some(UiAction::Invoke(ControlCommand::LoadSelectedSave))
        );
        assert_eq!(
            find("main_world_tool_view", "GridToggle").action,
            Some(UiAction::ToggleOverlay(OverlayKind::Grid))
        );
        let info_open_genes = controls
            .iter()
            .find(|control| {
                control.session == "main_world_information" && control.path.ends_with("/OpenGenes")
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing OpenGenes; info controls: {:?}",
                    controls
                        .iter()
                        .filter(|control| control.session == "main_world_information")
                        .map(|control| (&control.name, &control.path))
                        .collect::<Vec<_>>()
                )
            });
        assert_eq!(
            info_open_genes.action,
            Some(UiAction::SetTab(InspectorTab::Genes))
        );
        assert_eq!(
            find("main_world_information", "ExitButton").action,
            Some(UiAction::ClosePanel)
        );
        let _ = find("main_world_information", "InformationPanelButton");

        // Every active control in every functional session gets a hit rect.
        for name in functional {
            let expected = controls
                .iter()
                .filter(|control| control.session == name)
                .count();
            // One captured settings container has a control component but no
            // composed screen hit; keep it in the registry (for introspection)
            // but exclude it from the frame-hit count.
            let expected_hits =
                if name.starts_with("main_menu_settings") || name == "main_menu_bibite_library" {
                    expected.saturating_sub(1)
                } else {
                    expected
                };
            let frame = runtime
                .build_named_session_frame(name, &_atlas, 1920.0, 1080.0)
                .unwrap_or_else(|| panic!("failed to build frame for {name}"));
            // One world-space capture wrapper has no composed screen hit; allow
            // that single non-composable container while still requiring every
            // other registered control to be hit-testable.
            assert!(
                frame.hits.len() + 1 >= expected_hits,
                "hit coverage for {name}: {} vs {}",
                frame.hits.len(),
                expected_hits
            );
        }
    }
}

/// Current value of the boolean scenario setting whose game label is `label`
/// (`None` when no boolean setting matches).
fn live_scenario_flag(app: &App, label: &str) -> Option<(usize, bool)> {
    use bibites_core::settings::{ScenarioSettings, SettingValue};
    let wanted = label.trim().to_ascii_lowercase();
    let (index, meta) = ScenarioSettings::SETTINGS
        .iter()
        .enumerate()
        .find(|(_, meta)| meta.label.to_ascii_lowercase() == wanted)?;
    match app.world.settings.scenario.get_by_key(meta.key)? {
        SettingValue::Bool(flag) => Some((index, flag)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Native title-menu panels (New Game / Load Game)
// ---------------------------------------------------------------------------

/// Visible-row height of the scenario / save lists (reference px).
const MENU_LIST_ROW: f32 = 70.0;

/// How the live menu treats one captured node.
enum MenuRule {
    /// Draw normally.
    Show,
    /// Skip the node and its subtree.
    Hide,
    /// The node is replaced by the live list; its subtree is skipped.
    ReplaceList,
    /// The preview image placeholder: draw the scenario image.
    Preview,
    /// The challenge list's champion section takes the place of the scenario modifiers' panel (`showWithScenarios` / `showWithChallenges`).
    ChampionSection,
    /// The page's captured scrollbar (a still picture of the capture's state) is replaced by the live one (`emit_list_bar`).
    ListBar,
}

fn menu_option_group(path: &str) -> Option<u8> {
    let rest = path.split("basicDropdown").nth(1)?;
    if let Some(inner) = rest.strip_prefix(" (") {
        let digits: String = inner.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    } else {
        Some(0)
    }
}

fn menu_node_rule(menu: &crate::menu::MenuState, node: &Node) -> MenuRule {
    use crate::menu::MenuView;
    let path = node.path.as_str();
    match menu.view() {
        MenuView::NewGame => {
            let selected = menu.selected_scenario();
            let challenge = selected.is_some_and(|entry| entry.is_challenge);
            if path.ends_with("/ScenarioList/Scroll View/Viewport/Content") {
                return MenuRule::ReplaceList;
            }
            // The challenge list has no scenario modifiers: the champion section is in their place.
            if menu.showing_challenges() && path.ends_with("/RightGroup/ParametersPanel") {
                return MenuRule::ChampionSection;
            }
            if path.ends_with("/ScenarioInfo/preview/Image") {
                return MenuRule::Preview;
            }
            if path.ends_with("/ScenarioInfo/preview/No Preview")
                && selected.is_some_and(|entry| entry.preview_png.is_some())
            {
                return MenuRule::Hide;
            }
            if (path.ends_with("/Details/StarsDescriptions") || path.ends_with("/RightPanels/top"))
                && !challenge
            {
                return MenuRule::Hide;
            }
            if path.ends_with("/Details/prevHighscoreSection")
                && !menu
                    .challenge_best()
                    .is_some_and(|(_, stars, _)| *stars > 0)
            {
                return MenuRule::Hide;
            }
            if path.ends_with("/ParametersPanel/Warner") {
                return MenuRule::Hide;
            }
            MenuRule::Show
        }
        MenuView::Loading => {
            if path.contains("/Fill Area/Fill") {
                return MenuRule::Hide;
            }
            MenuRule::Show
        }
        MenuView::Library => {
            if path.ends_with("/TemplateTab/Scroll View/Viewport/Content/List") {
                return MenuRule::ReplaceList;
            }
            MenuRule::Show
        }
        MenuView::Settings | MenuView::Credits | MenuView::Tools => {
            if path.ends_with("/Scrollbar Vertical") {
                return MenuRule::ListBar;
            }
            MenuRule::Show
        }
        MenuView::LoadGame => {
            if path.ends_with("/LoadList/Scroll View/Viewport/Content/List") {
                return MenuRule::ReplaceList;
            }
            if path.ends_with("/Preview Holder/preview/Image") {
                return MenuRule::Preview;
            }
            if path.ends_with("/SelectedPanel/SaveInfo") && menu.save_card().is_none() {
                return MenuRule::Hide;
            }
            if path.ends_with("/Version/IncompatibleIndicator") {
                return MenuRule::Hide;
            }
            if path.ends_with("/SettingToggle/Toggle/Background/Checkmark")
                && !menu.include_autosaves()
            {
                return MenuRule::Hide;
            }
            MenuRule::Show
        }
        _ => MenuRule::Show,
    }
}

fn menu_live_text(menu: &crate::menu::MenuState, node: &Node, original: &str) -> String {
    use crate::menu::MenuView;
    let path = node.path.as_str();
    if menu.view() == MenuView::Settings {
        return original.to_owned();
    }
    if menu.view() == MenuView::Loading {
        return if path.ends_with("/ProgressText") {
            "0.00%".to_owned()
        } else {
            original.to_owned()
        };
    }
    if menu.view() == MenuView::Library {
        let entry = menu
            .library_selected()
            .and_then(|index| menu.templates().get(index));
        // `OpenWithCallBacks(..., submitText: "Select Bibite")`; a Library opened to edit (`OpenWithoutGoals`,
        // `OpenForBibiteEditor`) says "Edit Bibite".
        if path.contains("/BibitePreview/Bottom/Button") && !original.is_empty() {
            return if menu.pick().is_some() {
                "Select Bibite"
            } else {
                "Edit Bibite"
            }
            .to_owned();
        }
        // The two tabs share one list area here; the Saved tab's list has its own title.
        if path.ends_with("/TemplateTab/Scroll View/Viewport/Content/Title") {
            return if menu.library_tab() == 1 {
                "Saved Bibites".to_owned()
            } else {
                original.to_owned()
            };
        }
        if path.ends_with("/BibitePreview/Name") {
            return entry.map_or_else(|| original.to_owned(), |entry| entry.name.clone());
        }
        if path.ends_with("/BibitePreview/Desc") {
            return entry.map_or_else(|| original.to_owned(), |entry| entry.description.clone());
        }
        // `BibiteTemplateGenePreviewer.UpdateTemplate`: hidden nodes / synapses.
        if path.ends_with("/Brain line/nodes/FloatInfoText") {
            return entry.map_or_else(
                || original.to_owned(),
                |entry| entry.n_hidden_nodes.to_string(),
            );
        }
        if path.ends_with("/Brain line/synapses/FloatInfoText (1)") {
            return entry.map_or_else(|| original.to_owned(), |entry| entry.n_synapses.to_string());
        }
        return original.to_owned();
    }
    if menu.view() == MenuView::LoadGame {
        let Some(card) = menu.save_card() else {
            return original.to_owned();
        };
        // Only the selected row: this runs for every text of the screen, and a list can hold 1,500 saves.
        let selected_row = menu.selected_save_row();
        if path.ends_with("/Save Name") {
            return selected_row.map_or_else(String::new, |(name, _)| name);
        }
        if path.ends_with("/LastEdit/datetime") {
            return selected_row.map_or_else(String::new, |(_, date)| date);
        }
        if path.ends_with("/Version/version") {
            return card.version.clone().unwrap_or_default();
        }
        if path.ends_with("/Time/time") {
            let total = card.sim_time.max(0.0) as u64;
            return format!(
                "{:02}:{:02}:{:02}",
                total / 3_600,
                (total % 3_600) / 60,
                total % 60
            );
        }
        if path.ends_with("/Size/FloatInfoText") {
            return bibites_core::format::FloatValueFormat {
                units: "u",
                precision: 3,
                ..bibites_core::format::FloatValueFormat::default()
            }
            .format(card.sim_size);
        }
        if path.ends_with("/Bibites/bibites") {
            return card
                .n_bibites
                .map_or_else(|| "?".to_owned(), |count| count.to_string());
        }
        if path.ends_with("/Pellets/pellets") {
            return card.n_pellets.to_string();
        }
        return original.to_owned();
    }
    if menu.view() != MenuView::NewGame {
        return original.to_owned();
    }
    let selected = menu.selected_scenario();
    if path.ends_with("/Top/Screen Title") {
        return if menu.showing_challenges() {
            "Challenges"
        } else {
            "Scenarios"
        }
        .to_owned();
    }
    if path.ends_with("/Details/Name Line/Name") {
        return selected.map_or_else(String::new, |entry| entry.name.clone());
    }
    if path.ends_with("/Details/Description") {
        return selected.map_or_else(String::new, |entry| entry.description.clone());
    }
    for (index, marker) in ["1starLine", "2starLine", "3starLine"].iter().enumerate() {
        if path.contains(marker) && path.ends_with("/Text (TMP)") {
            return selected.map_or_else(String::new, |entry| entry.stars[index].clone());
        }
    }
    if path.ends_with("/prevHighScore") {
        return menu
            .challenge_best()
            .map_or_else(String::new, |(score, _, _)| format!("{score:.0}"));
    }
    if path.ends_with("/OldChampionName") {
        return menu
            .challenge_best()
            .map_or_else(String::new, |(_, _, champion)| champion.clone());
    }
    if path.ends_with("/Dropdown/Label")
        && let Some(group) = menu_option_group(path)
    {
        return menu
            .modifier_labels()
            .get(usize::from(group))
            .map_or_else(String::new, |label| (*label).to_owned());
    }
    original.to_owned()
}

fn menu_hit_action(menu: &crate::menu::MenuState, node: &Node) -> Option<UiAction> {
    use crate::menu::MenuView;
    use crate::ui::MenuCommand;
    let path = node.path.as_str();
    let name = node.name.as_str();
    match menu.view() {
        MenuView::NewGame => {
            if !path.contains("/NewGameScenarios/") {
                return None;
            }
            let command = if path.ends_with("/Top/Return") {
                MenuCommand::Back
            } else if name == "Challenges Buttons" {
                MenuCommand::ToggleChallenges
            } else if name == "New Scenario Button" {
                MenuCommand::NewScenario
            } else if name == "Open Folder Button" {
                MenuCommand::OpenFolder
            } else if name == "EditChallengeButton" || name == "Edit Button" {
                MenuCommand::EditScenario
            } else if name.trim() == "Performance" {
                MenuCommand::TpsSettings
            } else if name == "StartButton" {
                MenuCommand::StartSimulation
            } else if name == "Dropdown" {
                // The modifier's list hangs under the control (`TMP_Dropdown.Show`).
                return Some(UiAction::DropdownOpen(crate::ui::DropdownTarget::Modifier(
                    menu_option_group(path)?,
                )));
            } else {
                return None;
            };
            Some(UiAction::Menu(command))
        }
        MenuView::Achievements
        | MenuView::Tools
        | MenuView::Credits
        | MenuView::Library
        | MenuView::Settings => {
            let command = if path.contains("/ButtonsPanel/") {
                match name {
                    "Continue" => MenuCommand::MainButton(0),
                    "NewSim" => MenuCommand::MainButton(1),
                    "LoadSim" => MenuCommand::MainButton(2),
                    "Editor" => MenuCommand::MainButton(3),
                    "Settings" => MenuCommand::MainButton(4),
                    "Achievements" => MenuCommand::MainButton(5),
                    "More" => MenuCommand::MainButton(6),
                    "Credits" => MenuCommand::MainButton(7),
                    "Quit" => MenuCommand::MainButton(8),
                    _ => return None,
                }
            } else if name == "CloseButton" || name == "BackButton" {
                MenuCommand::Back
            } else if menu.view() == MenuView::Library {
                match name {
                    "Button" if path.contains("/BibitePreview/Bottom/") => {
                        MenuCommand::StartTemplate
                    }
                    // `OpenEditorForNewBibite` / `EditSelectedBibite`.
                    "CreateNew" => MenuCommand::NewTemplate,
                    "Edit Button" => MenuCommand::EditTemplate,
                    // `BibiteTemplateSelectorPanel`: tabs, `FillLists`, `OpenSaveFolder`.
                    "Templates" => MenuCommand::LibraryTab(0),
                    "Saved" => MenuCommand::LibraryTab(1),
                    "Reload " => MenuCommand::LibraryReload,
                    "OpenFolder" => MenuCommand::LibraryOpenFolder,
                    // `deleteTemplateButton.interactable`: not for the official bibites.
                    "Delete Button" if menu.library_can_delete() => MenuCommand::LibraryDelete,
                    "Delete Button" => return None,
                    _ => return None,
                }
            } else if path.contains("/ToolsPanel/")
                // The `Link` of a section is the `Button` (its `OnClick`); what answers the pointer is what it holds that is a graphic: the icon and the title.
                && (name == "Button" || (name == "Text (TMP)" && path.ends_with("/Link/Text (TMP)")))
            {
                if path.contains("/Wiki Section/") {
                    // `URLManager.OpenWiki`.
                    MenuCommand::OpenUrl(0)
                } else if path.contains("/Research Conglomerate/") {
                    // `URLManager.OpenResearchDiscord`.
                    MenuCommand::OpenUrl(1)
                } else if path.contains("/Patreon/") {
                    // `GameManager.OpenPatreonSimulation`: another scene.
                    MenuCommand::Unavailable(7)
                } else {
                    // `GameManager.OpenSpriteGenerator`: another scene.
                    MenuCommand::Unavailable(5)
                }
            } else if path.contains("/CreditsPanel/") && path.contains("/Socials Buttons/") {
                // The social buttons are `URLManager`'s `OpenReddit`, `OpenTwitter`, `OpenPatreon`, `OpenBlueSky` and `OpenYoutube`, in the order the
                // buttons are named (the layout puts them in another).
                match name {
                    "Button" => MenuCommand::OpenUrl(3),
                    "Button (1)" => MenuCommand::OpenUrl(4),
                    "Button (2)" => MenuCommand::OpenUrl(2),
                    "Button (3)" => MenuCommand::OpenUrl(5),
                    "Button (4)" => MenuCommand::OpenUrl(6),
                    _ => return None,
                }
            } else if path.contains("/CreditsPanel/")
                && path.ends_with("/Patreon Simulation Link/Button")
            {
                // `GameManager.OpenPatreonSimulation`: another scene.
                MenuCommand::Unavailable(7)
            } else if menu.view() == MenuView::Settings {
                match name {
                    "GraphicsButton" => MenuCommand::SettingsTab(0),
                    "Gameplay Button" => MenuCommand::SettingsTab(1),
                    "Controls Button" => MenuCommand::SettingsTab(2),
                    "Reset Warnings" => MenuCommand::ResetWarnings,
                    _ => return None,
                }
            } else {
                return None;
            };
            Some(UiAction::Menu(command))
        }
        MenuView::LoadGame => {
            if !path.contains("/LoadGamePanel/") {
                return None;
            }
            let command = if name == "LoadButton" {
                MenuCommand::LoadSelected
            } else if name == "CloseButton" {
                MenuCommand::Back
            } else if name == "DeleteButton" {
                MenuCommand::DeleteSave
            } else if name == "Toggle" && path.contains("/SettingToggle/") {
                MenuCommand::ToggleAutosaves
            } else if name == "Button" && path.contains("/SettingToggle/") {
                MenuCommand::Unavailable(3)
            } else {
                return None;
            };
            Some(UiAction::Menu(command))
        }
        _ => None,
    }
}

/// Draws the live save list into the (empty) captured list viewport: the prefab's `SaveItem` rows (`save_panel::draw_row`), 805 wide at x 90 and
/// 50 high, 5 apart, from y 140 (under the "Include autosaves" line) to the end of the view at 1005. Every button has Unity's default colours: no
/// highlight but the 245 grey of a hovered or clicked one.
fn emit_save_list(context: &mut FrameContext<'_>, menu: &crate::menu::MenuState) {
    let s = context.scale;
    let drawer = crate::species_panel::Drawer::new(s, context.ui, context.metrics, menu.cursor());
    let bar = menu.list_bar();
    // The viewport gives up the scrollbar's 17 when it is there (`AutoHideAndExpandViewport`).
    let (x, top, width, height) = (
        90.0_f32,
        140.0_f32,
        if bar.is_some() { 788.0_f32 } else { 805.0_f32 },
        865.0_f32,
    );
    let (row_h, pitch) = (50.0_f32, 55.0_f32);
    let selected = menu.selected_save();
    let at = menu.cursor() / s;
    for row in 0..menu.save_count() {
        let y = top + row as f32 * pitch - menu.list_scroll();
        if y < top - 0.5 || y + row_h > top + height + 0.5 {
            continue;
        }
        let Some((name, date)) = menu.save_row(row) else {
            continue;
        };
        let (name, date) = (&name, &date);
        let hovered = at.x >= x && at.x < x + width && at.y >= y && at.y < y + row_h;
        let tint = crate::save_panel::button_tint(true, false, hovered || selected == Some(row));
        crate::save_panel::draw_row(
            context.frame,
            &drawer,
            [x, y, width, row_h],
            name,
            date,
            tint,
        );
        let id = context.next_hit;
        context.next_hit += 1;
        context.frame.hit(
            id,
            Vec2::new(x * s, y * s),
            Vec2::new(width * s, row_h * s),
            UiAction::Menu(crate::ui::MenuCommand::SelectSave(row as u16)),
        );
    }
    emit_list_bar(context, menu);
}

/// The scrollbar of the list on screen (`MenuState::list_bar`): drawn over the list's right edge, its handle takes the press (a drag, followed by the
/// window loop while the button is held), the track above and under it a page each way.
fn emit_list_bar(context: &mut FrameContext<'_>, menu: &crate::menu::MenuState) {
    let Some(bar) = menu.list_bar() else {
        return;
    };
    let s = context.scale;
    let drawer = crate::species_panel::Drawer::new(s, context.ui, context.metrics, menu.cursor());
    let at = menu.cursor() / s;
    let handle = bar.handle();
    let hovered = at.x >= handle[0]
        && at.x < handle[0] + handle[2]
        && at.y >= handle[1]
        && at.y < handle[1] + handle[3];
    bar.draw(
        context.frame,
        &drawer,
        crate::save_panel::button_tint(true, menu.bar_dragging(), hovered),
    );
    let rects = [
        (
            [
                bar.track[0],
                bar.track[1],
                bar.track[2],
                handle[1] - bar.track[1],
            ],
            UiAction::Menu(crate::ui::MenuCommand::ListBarPage(-1)),
        ),
        (
            handle,
            UiAction::Menu(crate::ui::MenuCommand::ListBarHandle),
        ),
        (
            [
                bar.track[0],
                handle[1] + handle[3],
                bar.track[2],
                bar.track[1] + bar.track[3] - handle[1] - handle[3],
            ],
            UiAction::Menu(crate::ui::MenuCommand::ListBarPage(1)),
        ),
    ];
    for (rect, action) in rects {
        if rect[3] <= 0.0 {
            continue;
        }
        let id = context.next_hit;
        context.next_hit += 1;
        context.frame.hit(
            id,
            Vec2::new(rect[0] * s, rect[1] * s),
            Vec2::new(rect[2] * s, rect[3] * s),
            action,
        );
    }
}

/// Draws the live scenario list into the (empty) captured list viewport.
fn emit_menu_list(context: &mut FrameContext<'_>, menu: &crate::menu::MenuState) {
    let s = context.scale;
    let rows = menu.visible_entries();
    // Viewport (reference px, y down): x 26..749, y 108..1054; it gives up the scrollbar's 16 when the bar is there.
    let (x, top, height) = (26.0_f32, 108.0_f32, 946.0_f32);
    let width = 723.0_f32 - menu.list_bar().map_or(0.0, |bar| bar.track[2]);
    let selected = menu.selected_entry_index();
    for (row, entry_index) in rows.iter().enumerate() {
        let y = top + row as f32 * MENU_LIST_ROW - menu.list_scroll();
        if y < top - 1.0 || y + MENU_LIST_ROW - 8.0 > top + height {
            continue;
        }
        let Some(entry) = menu.scenario_entries().get(*entry_index) else {
            continue;
        };
        let is_selected = selected == Some(*entry_index);
        let pos = Vec2::new(x * s, y * s);
        let size = Vec2::new(width * s, (MENU_LIST_ROW - 8.0) * s);
        if let Some(record) = context.ui.pick(&["base_icon"]) {
            let tint: Rgba = if is_selected {
                [150, 190, 255, 255]
            } else {
                [226, 226, 226, 255]
            };
            context
                .frame
                .nine_slice_tinted(context.ui, record, pos, size, tint);
        } else {
            context.frame.rect(
                pos,
                size,
                if is_selected {
                    [150, 190, 255, 255]
                } else {
                    [226, 226, 226, 255]
                },
            );
        }
        context.frame.bold(
            entry.name.as_str(),
            Vec2::new((x + 18.0) * s, (y + 12.0) * s),
            28.0 * s,
            [20, 20, 20, 255],
        );
        context.frame.text(
            entry.version.as_str(),
            Vec2::new((x + width - 90.0) * s, (y + 20.0) * s),
            18.0 * s,
            [70, 70, 70, 255],
        );
        if entry.is_official {
            context.frame.text(
                "official",
                Vec2::new((x + width - 190.0) * s, (y + 20.0) * s),
                18.0 * s,
                [60, 110, 60, 255],
            );
        }
        let id = context.next_hit;
        context.next_hit += 1;
        let clip_top = top.max(y);
        let clip_bottom = (y + MENU_LIST_ROW - 8.0).min(top + height);
        context.frame.hit(
            id,
            Vec2::new(x * s, clip_top * s),
            Vec2::new(width * s, (clip_bottom - clip_top).max(1.0) * s),
            UiAction::Menu(crate::ui::MenuCommand::SelectScenario(row as u16)),
        );
    }
    emit_list_bar(context, menu);
}

/// Draws the selected scenario's preview image over the placeholder.
fn emit_menu_preview(
    context: &mut FrameContext<'_>,
    menu: &crate::menu::MenuState,
    composed: ComposedRect,
) {
    let preview = if menu.view() == crate::menu::MenuView::LoadGame {
        menu.save_preview()
    } else {
        menu.selected_preview()
    };
    if let Some(texture) = preview {
        context.frame.draws.push(UiDraw::Pixels {
            texture,
            pos: composed.pos * context.scale,
            size: composed.size * context.scale,
        });
    }
}

impl RuntimeUi {
    /// The game's loading panel (title text, spinner, progress bar) over the title
    /// background, shown while a world opens.
    #[must_use]
    pub fn build_menu_loading_frame(
        &self,
        menu: &crate::menu::MenuState,
        ui: &UiAssets,
        width: f32,
        height: f32,
        metrics: Option<&crate::text_sdf::SdfFonts>,
    ) -> Option<UiFrame> {
        let mut session = self.menu_title.as_ref()?.clone();
        session
            .canvases
            .retain(|canvas| canvas.path == "LoadingPanel");
        let frame = self.build_session_frame_filtered(
            "main_menu_title",
            &session,
            None,
            metrics,
            ui,
            width,
            height,
            None,
            100,
            Some(menu),
        );
        let mut frame = frame;
        if !menu.video_background() {
            let background = ui.texture_of_alias("menu_background").map_or(
                UiDraw::Rect {
                    pos: Vec2::ZERO,
                    size: Vec2::new(width, height),
                    color: [18, 20, 22, 255],
                },
                |texture| UiDraw::Image {
                    texture,
                    pos: Vec2::ZERO,
                    size: Vec2::new(width, height),
                    tint: [255, 255, 255, 255],
                },
            );
            frame.draws.insert(0, background);
        }
        Some(frame)
    }

    /// The named session with the menu-only pruning applied, cached (the session tree has
    /// thousands of nodes; cloning it per frame was the dominant cost of menu clicks).
    fn filtered_menu_session(&self, name: &str) -> Option<std::sync::Arc<RuntimeSession>> {
        let mut cache = self.filtered_menu.lock().ok()?;
        if let Some(found) = cache.get(name) {
            return Some(std::sync::Arc::clone(found));
        }
        let filtered = std::sync::Arc::new(filter_menu_session(self.session_by_name(name)?));
        cache.insert(name.to_owned(), std::sync::Arc::clone(&filtered));
        Some(filtered)
    }

    /// Builds a native title-menu panel (New Game / Load Game) from its captured
    /// session with the menu's live data. `None` when the session is missing.
    #[must_use]
    pub fn build_menu_view_frame(
        &self,
        menu: &crate::menu::MenuState,
        ui: &UiAssets,
        width: f32,
        height: f32,
        metrics: Option<&crate::text_sdf::SdfFonts>,
    ) -> Option<UiFrame> {
        use crate::menu::MenuView;
        if menu.view() == MenuView::ScenarioEditor {
            return self.build_scenario_editor_frame(menu, ui, width, height, metrics);
        }
        let name = match menu.view() {
            MenuView::NewGame => "main_menu_new_game",
            MenuView::LoadGame => "main_menu_load_game",
            MenuView::Achievements => "main_menu_achievements",
            MenuView::Tools => "main_menu_additional_tools",
            MenuView::Credits => "main_menu_credits",
            MenuView::Library => "main_menu_bibite_library",
            MenuView::Settings => match menu.settings_tab() {
                1 => "main_menu_settings_gameplay",
                2 => "main_menu_settings_hotkeys",
                _ => "main_menu_settings_graphics",
            },
            MenuView::Main | MenuView::Loading | MenuView::ScenarioEditor => return None,
        };
        let session = self.filtered_menu_session(name)?;
        let mut frame = self.build_session_frame_filtered(
            name,
            &session,
            None,
            metrics,
            ui,
            width,
            height,
            None,
            100,
            Some(menu),
        );
        let s = UiFrame::scaler(width, height);
        let side_panel = !matches!(menu.view(), MenuView::NewGame | MenuView::LoadGame);
        // `BibiteSelector` (a plain Image, colour (0, 0, 0.106, 1)) and `SettingsPanel` (Image `Background`, (0, 0, 0.122, 1)) fill the
        // whole screen with an opaque navy, in the title scene and in the world alike: no title video and no (dimmed) world or HUD
        // shows through their margins. (The other side panels, Credits and Additional Tools, only cover their own column.)
        let opaque_root = match menu.view() {
            MenuView::Library => Some([0, 0, 27, 255]),
            MenuView::Settings => Some([0, 0, 31, 255]),
            _ => None,
        };
        if let Some(color) = opaque_root {
            frame.draws.insert(
                0,
                UiDraw::Rect {
                    pos: Vec2::ZERO,
                    size: Vec2::new(width, height),
                    color,
                },
            );
        } else if side_panel {
            // The title screen stays visible behind the side panels (in-game: the paused
            // world, dimmed).
            let overlay = menu.overlay();
            let background = ui
                .texture_of_alias("menu_background")
                .filter(|_| !overlay)
                .map_or(
                    UiDraw::Rect {
                        pos: Vec2::ZERO,
                        size: Vec2::new(width, height),
                        color: if overlay {
                            [0, 0, 0, 170]
                        } else {
                            [18, 20, 22, 255]
                        },
                    },
                    |texture| UiDraw::Image {
                        texture,
                        pos: Vec2::ZERO,
                        size: Vec2::new(width, height),
                        tint: [255, 255, 255, 255],
                    },
                );
            if !menu.video_background() {
                frame.draws.insert(0, background);
            }
            if matches!(menu.view(), MenuView::Achievements | MenuView::Credits) {
                // These panels have no close button in the capture: top-right corner.
                frame.hit(
                    42_000,
                    Vec2::new(width - 110.0 * s, 0.0),
                    Vec2::new(110.0 * s, 110.0 * s),
                    UiAction::Menu(crate::ui::MenuCommand::Back),
                );
            }
        } else if menu.overlay() {
            // In-game Load Game panel: the paused world stays visible, dimmed.
            frame.draws.insert(
                0,
                UiDraw::Rect {
                    pos: Vec2::ZERO,
                    size: Vec2::new(width, height),
                    color: [0, 0, 0, 170],
                },
            );
        }
        if menu.view() == MenuView::NewGame && menu.tps_open() {
            emit_tps_panel(&mut frame, ui, metrics, menu, s);
        }
        if let Some((source, text)) = menu.error_popup() {
            crate::error_popup::emit(
                &mut frame,
                ui,
                metrics,
                width,
                height,
                source,
                text,
                1.0,
                UiAction::Menu(crate::ui::MenuCommand::DismissError),
                41_500,
            );
        }
        if menu.confirm_library_delete() {
            // `ActionWarnings.deleteBibiteFileActionWarning`.
            let warning = crate::warnings::ActionWarning::DeleteBibiteFile;
            crate::warning_popup::emit(
                &mut frame,
                ui,
                metrics,
                width,
                height,
                &crate::warning_popup::Spec {
                    title: warning.shown_title(),
                    text: warning.text(),
                    proceed: warning.proceed_label(),
                    checked: menu.confirm_checked(),
                    scale: 1.0,
                },
                crate::warning_popup::Actions {
                    cancel: UiAction::Menu(crate::ui::MenuCommand::ConfirmLibraryDelete(false)),
                    proceed: UiAction::Menu(crate::ui::MenuCommand::ConfirmLibraryDelete(true)),
                    toggle: UiAction::Menu(crate::ui::MenuCommand::ToggleDoNotShowAgain),
                },
                41_100,
            );
        }
        if menu.confirm_delete() {
            // `ActionWarnings.deleteSaveFileActionWarning`: the warning popup (the title menu's
            // `Popuholder` is not scaled).
            let warning = crate::warnings::ActionWarning::DeleteSaveFile;
            crate::warning_popup::emit(
                &mut frame,
                ui,
                metrics,
                width,
                height,
                &crate::warning_popup::Spec {
                    title: warning.shown_title(),
                    text: warning.text(),
                    proceed: warning.proceed_label(),
                    checked: menu.confirm_checked(),
                    scale: 1.0,
                },
                crate::warning_popup::Actions {
                    cancel: UiAction::Menu(crate::ui::MenuCommand::ConfirmDelete(false)),
                    proceed: UiAction::Menu(crate::ui::MenuCommand::ConfirmDelete(true)),
                    toggle: UiAction::Menu(crate::ui::MenuCommand::ToggleDoNotShowAgain),
                },
                41_000,
            );
        }
        if !menu.status().is_empty() {
            frame.text(
                menu.status(),
                Vec2::new(24.0 * s, (height / s - 34.0) * s),
                20.0 * s,
                [255, 225, 140, 255],
            );
        }
        Some(frame)
    }
}

/// The zone shapes of the scenario editor's preview (`ZonePreview`): flat fills with the zone
/// shader's colour and opacity, clipped to the square.
fn emit_editor_zone_preview(
    frame: &mut UiFrame,
    shapes: &[crate::scenario_editor::PreviewShape],
    base: Rgba,
    scale: f32,
) {
    /// The preview square (reference px): `ZonePreview` under `Preview Holder`.
    const PARENT: [f32; 4] = [1173.0, 190.0, 1880.0, 897.0];
    emit_zone_preview(frame, shapes, base, PARENT, scale);
}

/// The zone shapes of a `ZonePreview`: flat fills with the zone shader's colour and opacity on the
/// square `PARENT` (reference px, `x0, y0, x1, y1`), clipped to it.
pub(crate) fn emit_zone_preview(
    frame: &mut UiFrame,
    shapes: &[crate::scenario_editor::PreviewShape],
    base: Rgba,
    parent: [f32; 4],
    scale: f32,
) {
    use crate::scenario_editor::PreviewKind;
    let (parent_w, parent_h) = (parent[2] - parent[0], parent[3] - parent[1]);
    let (mid_x, mid_y) = (parent[0] + parent_w * 0.5, parent[1] + parent_h * 0.5);
    for shape in shapes {
        let cx = mid_x + shape.centre[0] * parent_w * 0.5;
        let cy = mid_y + shape.centre[1] * parent_h * 0.5;
        let (w, h) = (shape.size[0] * parent_w, shape.size[1] * parent_w);
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let x0 = (cx - w * 0.5).max(parent[0]);
        let y0 = (cy - h * 0.5).max(parent[1]);
        let x1 = (cx + w * 0.5).min(parent[2]);
        let y1 = (cy + h * 0.5).min(parent[3]);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let tex_w = ((x1 - x0) * scale).ceil().max(1.0) as u32;
        let tex_h = ((y1 - y0) * scale).ceil().max(1.0) as u32;
        let colour = [
            (f32::from(base[0]) * shape.palette)
                .round()
                .clamp(0.0, 255.0) as u8,
            (f32::from(base[1]) * shape.palette)
                .round()
                .clamp(0.0, 255.0) as u8,
            (f32::from(base[2]) * shape.palette)
                .round()
                .clamp(0.0, 255.0) as u8,
        ];
        let alpha = (shape.opacity * 255.0).round().clamp(0.0, 255.0);
        let edge = (w.min(h) * 0.5 * scale).max(1.0);
        let mut rgba = vec![0_u8; (tex_w * tex_h * 4) as usize];
        for py in 0..tex_h {
            for px in 0..tex_w {
                let rx = x0 + (px as f32 + 0.5) / scale;
                let ry = y0 + (py as f32 + 0.5) / scale;
                let coverage = match shape.kind {
                    PreviewKind::Rect => 1.0,
                    PreviewKind::Disc | PreviewKind::Ring { .. } => {
                        let nx = (rx - cx) / (w * 0.5);
                        let ny = (ry - cy) / (h * 0.5);
                        let distance = (nx * nx + ny * ny).sqrt();
                        let mut coverage = ((1.0 - distance) * edge).clamp(0.0, 1.0);
                        if let PreviewKind::Ring { inside } = shape.kind {
                            coverage = coverage.min(((distance - inside) * edge).clamp(0.0, 1.0));
                        }
                        coverage
                    }
                };
                if coverage <= 0.0 {
                    continue;
                }
                let at = ((py * tex_w + px) * 4) as usize;
                rgba[at..at + 3].copy_from_slice(&colour);
                rgba[at + 3] = (alpha * coverage).round() as u8;
            }
        }
        frame.draws.push(UiDraw::Pixels {
            texture: std::sync::Arc::new(crate::ui::UiTexture {
                width: tex_w,
                height: tex_h,
                rgba,
            }),
            pos: Vec2::new(x0 * scale, y0 * scale),
            size: Vec2::new(tex_w as f32, tex_h as f32),
        });
    }
}

/// Greedy word wrap: breaks `text` so no line is wider than `max_width`.
pub(crate) fn wrap_words(text: &str, max_width: f32, measure: &dyn Fn(&str) -> f32) -> String {
    let mut out = String::new();
    for (index, paragraph) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && measure(&candidate) > max_width {
                out.push_str(&line);
                out.push('\n');
                line = word.to_owned();
            } else {
                line = candidate;
            }
        }
        out.push_str(&line);
    }
    out
}

/// Label of a Settings-screen row container (`SettingToggle(Clone)`, ...): the
/// text of its `Name` child.
fn settings_row_label(node: &Node) -> Option<String> {
    let is_row = [
        "SettingToggle",
        "SubSettingMasterTogglePrefab",
        "SettingDropdown",
        "SettingSlider",
        "ColorPicker",
    ]
    .iter()
    .any(|prefix| node.name.starts_with(prefix));
    if !is_row {
        return None;
    }
    node.children
        .iter()
        .find(|child| child.name == "Name")
        .and_then(|child| child.texts.first())
        .map(|text| text.text.clone())
}

/// Live text of a Settings-screen value label (dropdown label, slider value,
/// colour code).
fn settings_live_text(
    menu: &crate::menu::MenuState,
    row: Option<usize>,
    node: &Node,
    original: &str,
) -> Option<String> {
    if menu.view() != crate::menu::MenuView::Settings {
        return None;
    }
    let index = row?;
    let path = node.path.as_str();
    if path.ends_with("/Dropdown/Label")
        || path.ends_with("/FloatInfoText")
        || path.contains("/ColorDisplay")
    {
        let text = menu.user_settings().display(index);
        return if text.is_empty() { None } else { Some(text) };
    }
    let _ = original;
    None
}

/// The top-centre time-speed slider (`LogFloatSettingSlider(targetTimeScale, 5)`): the fill and
/// handle follow the live speed, and its panel is a draggable control.
fn speed_slider_geometry(
    context: &mut FrameContext<'_>,
    node: &Node,
    composed: ComposedRect,
) -> ComposedRect {
    const BASE: &str = "/Controls/TopCenter/sliderHolder/BasicSlider/NormalSecton/SliderPanel";
    let Some(app) = context.app else {
        return composed;
    };
    if !node.path.contains(BASE) || crate::dyn_panel::is_generated(&context.session) {
        return composed;
    }
    // Track in the capture (reference px): x 845..1075.
    const TRACK: (f32, f32) = (845.0, 1075.0);
    let fraction = App::speed_fraction(app.speed);
    let centre = TRACK.0 + fraction * (TRACK.1 - TRACK.0);
    let mut result = composed;
    if node.path.ends_with("/Slider/Fill Area/Fill") {
        result.size.x = (centre + 1.0 - composed.pos.x).max(0.0);
    } else if node.path.ends_with("/Handle Slide Area/Handle") {
        result.pos.x = centre - composed.size.x * 0.5;
    }
    result
}

/// Moves a slider's fill and handle to the setting's current value.
fn settings_slider_geometry(
    context: &FrameContext<'_>,
    node: &Node,
    composed: ComposedRect,
) -> ComposedRect {
    let (Some(menu), Some(index)) = (context.menu, context.setting_row) else {
        return composed;
    };
    if menu.view() != crate::menu::MenuView::Settings || !node.path.contains("/SettingSlider") {
        return composed;
    }
    // Slider track in the captured layout: x 822..1157 (reference px).
    const TRACK_X: f32 = 822.0;
    const TRACK_W: f32 = 335.0;
    let fraction = menu.user_settings().fraction(index);
    let mut result = composed;
    if node.path.ends_with("/Slider/Fill Area/Fill") {
        result.size.x = 5.0 + fraction * TRACK_W;
    } else if node.path.ends_with("/Handle Slide Area/Handle") {
        result.pos.x = TRACK_X + fraction * TRACK_W - composed.size.x * 0.5;
    }
    result
}

/// Where a node of a scrolling panel is against the panel's view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InView {
    /// All of it (every node of a panel that does not scroll, too).
    Whole,
    /// A part of a text: the lines of it inside the view are drawn (`FrameContext::scroll_clip`), nothing else of the node.
    Part,
    /// None of it, or a part of something that cannot be cut.
    Out,
}

/// A title-menu panel whose content scrolls (a `ScrollRect` with a `Mask`): the nodes under its `Viewport/Content` were captured with the content
/// `captured` px scrolled, in a view whose rows are `top` to `bottom` (reference px, y down).
struct ScrollView {
    captured: f32,
    top: f32,
    bottom: f32,
    /// The texts are cut line by line, the way the mask cuts them (the Settings rows are single lines: they stay or go with their row).
    by_lines: bool,
}

fn scroll_view(view: crate::menu::MenuView) -> Option<ScrollView> {
    use crate::menu::MenuView;
    match view {
        MenuView::Settings => Some(ScrollView {
            captured: 0.0,
            top: 79.0,
            bottom: 1001.0,
            by_lines: false,
        }),
        // The Credits tab was captured as the game opens it: scrolled 622.96 (its prefab stores that), the Additional Tools tab at its top.
        MenuView::Credits => Some(ScrollView {
            captured: crate::menu::CREDITS_START_SCROLL,
            top: 99.0,
            bottom: 1056.0,
            by_lines: true,
        }),
        MenuView::Tools => Some(ScrollView {
            captured: 0.0,
            top: 99.0,
            bottom: 1056.0,
            by_lines: true,
        }),
        _ => None,
    }
}

/// Scrolls the panels of the title menu that have a `ScrollRect` (the Settings pages, the Credits and Additional Tools tabs): the content's nodes move by
/// the way the scroll differs from the captured one, and are left out where the view's mask would hide them.
fn panel_scroll(
    context: &mut FrameContext<'_>,
    node: &Node,
    composed: ComposedRect,
) -> (ComposedRect, InView) {
    context.scroll_clip = None;
    let Some(menu) = context.menu else {
        return (composed, InView::Whole);
    };
    let Some(view) = scroll_view(menu.view()) else {
        return (composed, InView::Whole);
    };
    if !node.path.contains("/Viewport/Content/") {
        return (composed, InView::Whole);
    }
    let mut shifted = composed;
    shifted.pos.y += view.captured - menu.list_scroll();
    let (top, bottom) = (shifted.pos.y, shifted.pos.y + shifted.size.y);
    let whole = top >= view.top && bottom <= view.bottom;
    let in_view = if whole {
        InView::Whole
    } else if view.by_lines && !node.texts.is_empty() && bottom > view.top && top < view.bottom {
        context.scroll_clip = Some([view.top, view.bottom]);
        InView::Part
    } else {
        InView::Out
    };
    if whole && view.by_lines && !node.texts.is_empty() {
        context.scroll_clip = Some([view.top, view.bottom]);
    }
    (shifted, in_view)
}

/// The New Game screen's "TPS Settings" popup (`ScenarioSelectorPanel.tpsSettings`): five
/// log sliders (`LogIntSettingSlider`, base 2) over the parameters panel. The capture only
/// has the inactive hierarchy (texts, no geometry), so the layout follows the `SettingSlider`
/// row anatomy captured in the Settings screens.
fn emit_tps_panel(
    frame: &mut UiFrame,
    ui: &UiAssets,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    menu: &crate::menu::MenuState,
    s: f32,
) {
    use crate::menu::{MenuState, TPS_SLIDERS};
    // Parameters panel (reference px, y down).
    let (px0, py0, pw, ph) = (777.0_f32, 587.0_f32, 1133.0_f32, 483.0_f32);
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let dim = |w: f32, h: f32| Vec2::new(w * s, h * s);
    let white: Rgba = [245, 245, 245, 255];
    // Blocks clicks on the controls underneath.
    frame.hit(42_100, at(px0, py0), dim(pw, ph), UiAction::Consume);
    frame.rect(at(px0, py0), dim(pw, ph), [236, 236, 236, 255]);
    frame.rect(
        at(px0 + 3.0, py0 + 3.0),
        dim(pw - 6.0, ph - 6.0),
        [38, 38, 38, 255],
    );
    // Title line.
    frame.bold(
        "Simulation Performance Settings",
        at(px0 + 25.0, py0 + 14.0),
        36.0 * s,
        white,
    );
    let button = |frame: &mut UiFrame, x: f32, y: f32, w: f32, h: f32| {
        frame.rect(at(x, y), dim(w, h), [20, 20, 20, 255]);
        frame.rect(
            at(x + 2.0, y + 2.0),
            dim(w - 4.0, h - 4.0),
            [236, 236, 236, 255],
        );
    };
    let icon = |frame: &mut UiFrame, names: &[&str], pos: Vec2, size: Vec2| -> bool {
        for name in names {
            if let Some(texture) = ui.texture_of_alias(name) {
                frame.draws.push(UiDraw::Image {
                    texture,
                    pos,
                    size,
                    tint: [255, 255, 255, 255],
                });
                return true;
            }
        }
        false
    };
    // "Save as Default" and the close button.
    let (save_x, save_y, save_w, save_h) = (px0 + pw - 380.0, py0 + 14.0, 260.0, 46.0);
    button(frame, save_x, save_y, save_w, save_h);
    frame.bold(
        "Save as Default",
        at(save_x + 22.0, save_y + 8.0),
        26.0 * s,
        [30, 30, 30, 255],
    );
    frame.hit(
        42_101,
        at(save_x, save_y),
        dim(save_w, save_h),
        UiAction::Menu(crate::ui::MenuCommand::TpsSaveDefault),
    );
    let (quit_x, quit_y) = (px0 + pw - 64.0, py0 + 12.0);
    button(frame, quit_x, quit_y, 50.0, 50.0);
    icon(
        frame,
        &["exit_icon"],
        at(quit_x + 5.0, quit_y + 5.0),
        dim(40.0, 40.0),
    );
    frame.hit(
        42_102,
        at(quit_x, quit_y),
        dim(50.0, 50.0),
        UiAction::Menu(crate::ui::MenuCommand::TpsSettings),
    );
    // Description (`TPS Settings/Desc`).
    let desc = "The simulation runs on a tick-based system and is independant from rendering (fps)\n\
Everything from the physics, to the different simulated systems, to other processes, are updated every tick, at the frequency specified here. \n\
Some systems, like vision, or the bibites brain updates, can be specified at a mutliple of tick to improve performances.";
    let desc_px = 20.0 * s;
    let measure = |line: &str| -> f32 {
        metrics.map_or(line.chars().count() as f32 * desc_px * 0.52, |metrics| {
            metrics.measure_named(None, FontRole::Body, line, desc_px)
        })
    };
    let wrapped = wrap_words(desc, (pw - 50.0) * s, &measure);
    for (index, line) in wrapped.lines().enumerate() {
        frame.text(
            line,
            at(px0 + 25.0, py0 + 72.0 + index as f32 * 24.0),
            desc_px,
            [215, 215, 215, 255],
        );
    }
    // Slider rows.
    let values = menu.tps_values();
    let defaults = menu.tps_defaults();
    let row_x = px0 + 25.0;
    let handle_half = 15.0_f32;
    for (index, (label, units, _, _)) in TPS_SLIDERS.iter().enumerate() {
        let ry = py0 + 208.0 + index as f32 * 54.0;
        frame.text(*label, at(row_x, ry + 7.0), 25.0 * s, white);
        frame.text(
            format!("{}{units}", values[index]),
            at(row_x + 415.0, ry + 7.0),
            25.0 * s,
            white,
        );
        // Slider panel + track (`NormalSecton/SliderPanel/Slider`).
        let (panel_x, panel_w) = (row_x + 615.0, 355.0_f32);
        if let Some(record) = ui.pick(&["Background"]) {
            frame.nine_slice_tinted(
                ui,
                record,
                at(panel_x, ry),
                dim(panel_w, 40.0),
                [255, 255, 255, 100],
            );
            frame.nine_slice_tinted(
                ui,
                record,
                at(panel_x + 10.0, ry + 10.0),
                dim(panel_w - 20.0, 20.0),
                [255, 255, 255, 64],
            );
        } else {
            frame.rect(
                at(panel_x + 10.0, ry + 10.0),
                dim(panel_w - 20.0, 20.0),
                [120, 120, 120, 255],
            );
        }
        let (track_x0, track_x1) = (
            panel_x + 10.0 + handle_half,
            panel_x + panel_w - 10.0 - handle_half,
        );
        let fraction = MenuState::tps_fraction_of(index, values[index]);
        let handle_x = track_x0 + fraction * (track_x1 - track_x0);
        // Fill (`Fill Area/Fill`) up to the handle.
        let fill_w = (handle_x - (panel_x + 10.0)).max(4.0);
        if let Some(record) = ui.pick(&["UISprite"]) {
            frame.nine_slice_tinted(
                ui,
                record,
                at(panel_x + 10.0, ry + 10.0),
                dim(fill_w, 20.0),
                [255, 255, 255, 255],
            );
        } else {
            frame.rect(
                at(panel_x + 10.0, ry + 10.0),
                dim(fill_w, 20.0),
                [255, 255, 255, 255],
            );
        }
        // Default-value pointer (`defaultvaluepointer`).
        let default_x =
            track_x0 + MenuState::tps_fraction_of(index, defaults[index]) * (track_x1 - track_x0);
        frame.rect(
            at(default_x - 2.0, ry + 10.0),
            dim(4.0, 20.0),
            [245, 245, 245, 255],
        );
        frame.rect(
            at(default_x - 2.0, ry + 30.0),
            dim(4.0, 10.0),
            [245, 245, 245, 255],
        );
        frame.rect(
            at(default_x - 2.0, ry),
            dim(4.0, 10.0),
            [245, 245, 245, 255],
        );
        // Handle.
        if !icon(
            frame,
            &["base_circle_icon"],
            at(handle_x - 15.0, ry + 5.0),
            dim(30.0, 30.0),
        ) {
            frame.rect(
                at(handle_x - 15.0, ry + 5.0),
                dim(30.0, 30.0),
                [255, 255, 255, 255],
            );
        }
        frame.hit(
            42_110 + index as UiId,
            at(panel_x, ry),
            dim(panel_w, 40.0),
            UiAction::Menu(crate::ui::MenuCommand::TpsSlide(
                index as u8,
                (track_x0 * s).round() as u16,
                (track_x1 * s).round() as u16,
            )),
        );
        // Reset button (`NormalSecton/ResetButton`).
        let reset_x = row_x + 1043.0;
        button(frame, reset_x, ry, 40.0, 40.0);
        icon(
            frame,
            &["Reset settings icon-1.png", "reset_icon"],
            at(reset_x - 2.0, ry - 2.0),
            dim(44.0, 44.0),
        );
        frame.hit(
            42_120 + index as UiId,
            at(reset_x, ry),
            dim(40.0, 40.0),
            UiAction::Menu(crate::ui::MenuCommand::TpsReset(index as u8)),
        );
    }
}

/// Draws the Bibite Library template list into the captured list area.
fn emit_template_list(context: &mut FrameContext<'_>, menu: &crate::menu::MenuState) {
    let s = context.scale;
    // List area (reference px, y down): x 41..1111, y 173..1054; it gives up the scrollbar's 12 when the bar is there.
    let (x, top, height) = (41.0_f32, 173.0_f32, 880.0_f32);
    let width = 1070.0_f32 - menu.list_bar().map_or(0.0, |bar| bar.track[2]);
    let selected = menu.library_selected();
    // The open tab's files; a row selects its entry of the whole list.
    for (position, row) in menu.library_rows().into_iter().enumerate() {
        let Some(entry) = menu.templates().get(row) else {
            continue;
        };
        let y = top + position as f32 * 56.0 - menu.list_scroll();
        if y < top - 1.0 || y + 50.0 > top + height {
            continue;
        }
        let is_selected = selected == Some(row);
        let pos = Vec2::new(x * s, y * s);
        let size = Vec2::new(width * s, 50.0 * s);
        let tint: Rgba = if is_selected {
            [150, 190, 255, 255]
        } else {
            [226, 226, 226, 255]
        };
        if let Some(record) = context.ui.pick(&["base_icon"]) {
            context
                .frame
                .nine_slice_tinted(context.ui, record, pos, size, tint);
        } else {
            context.frame.rect(pos, size, tint);
        }
        context.frame.bold(
            entry.name.as_str(),
            Vec2::new((x + 16.0) * s, (y + 9.0) * s),
            26.0 * s,
            [20, 20, 20, 255],
        );
        // `BibiteItem`: the file's version on the right.
        context.frame.text(
            entry.version.as_str(),
            Vec2::new((x + width - 110.0) * s, (y + 17.0) * s),
            18.0 * s,
            [70, 70, 70, 255],
        );
        let id = context.next_hit;
        context.next_hit += 1;
        context.frame.hit(
            id,
            pos,
            size,
            UiAction::Menu(crate::ui::MenuCommand::SelectTemplate(row as u16)),
        );
    }
    emit_list_bar(context, menu);
}

#[cfg(test)]
mod title_hit_probe {
    use super::*;

    /// Probe: the rectangles `hit_row` tests clicks against (`menu_button_rects`) and the buttons the title frame draws, at the sizes a window can be.
    #[test]
    fn the_title_buttons_hit_where_they_are_drawn_at_any_window_size() {
        let Ok(Some(runtime)) = RuntimeUi::discover() else {
            return;
        };
        let Ok(Some(ui)) = UiAssets::discover() else {
            return;
        };
        for (scale, (w, h)) in [1.0_f32, 0.5, 0.75, 1.15, 1.25, 1.5].into_iter().flat_map(|scale| {
            [(1280.0, 720.0), (1920.0, 1080.0), (2560.0, 1440.0), (2048.0, 1152.0), (1706.0, 960.0), (3840.0, 2160.0), (2560.0, 1080.0)]
                .into_iter()
                .map(move |size| (scale, size))
        }) {
            crate::ui::set_ui_scale_settings(true, 1.0, scale);
            eprintln!("ui scale {scale}");
            let frame = runtime.build_menu_frame(false, &ui, w, h, None).expect("title frame");
            let hit_rects = runtime.menu_button_rects(w, h);
            assert_eq!(hit_rects.len(), 9);
            // The drawn buttons: the frame's hit rectangles that are button sized.
            let mut worst = 0.0_f32;
            for (pos, size) in &hit_rects {
                let centre = Vec2::new(pos.x + size.x / 2.0, pos.y + size.y / 2.0);
                // Some drawn hit rectangle must contain this centre and have about this size.
                let found = frame.hits.iter().any(|(_, p, s)| {
                    centre.x >= p.x && centre.x <= p.x + s.x && centre.y >= p.y && centre.y <= p.y + s.y
                        && (s.x - size.x).abs() < 2.0 && (s.y - size.y).abs() < 2.0
                });
                if !found {
                    let nearest = frame.hits.iter().map(|(_, p, s)| ((p.x - pos.x).abs() + (p.y - pos.y).abs(), *p, *s)).min_by(|a, b| a.0.total_cmp(&b.0));
                    worst = worst.max(nearest.map_or(9999.0, |n| n.0));
                    eprintln!("{w}x{h}: hit rect {pos:?} {size:?} has no drawn twin; nearest drawn {nearest:?}");
                }
            }
            eprintln!("{w}x{h}: worst offset {worst}");
            // The drawn images (the buttons' sprites): is there one at each hit rectangle?
            let mut missing = 0;
            for (pos, size) in &hit_rects {
                let drawn = frame.draws.iter().any(|draw| match draw {
                    UiDraw::NineSlice { pos: p, size: s, .. } | UiDraw::Image { pos: p, size: s, .. } => {
                        (p.x - pos.x).abs() < 3.0 && (p.y - pos.y).abs() < 3.0 && (s.x - size.x).abs() < 3.0 && (s.y - size.y).abs() < 3.0
                    }
                    _ => false,
                });
                if !drawn {
                    missing += 1;
                    if missing == 1 {
                        let mut near: Vec<_> = frame
                            .draws
                            .iter()
                            .filter_map(|draw| match draw {
                                UiDraw::NineSlice { pos: p, size: s, .. } | UiDraw::Image { pos: p, size: s, .. } => {
                                    Some(((p.y - pos.y).abs() + (s.y - size.y).abs() * 0.2, *p, *s))
                                }
                                _ => None,
                            })
                            .collect();
                        near.sort_by(|a, b| a.0.total_cmp(&b.0));
                        let same_size: Vec<_> = frame
                            .draws
                            .iter()
                            .filter_map(|draw| match draw {
                                UiDraw::NineSlice { pos: p, size: s, .. } | UiDraw::Image { pos: p, size: s, .. }
                                    if (s.x - size.x).abs() < 1.0 && (s.y - size.y).abs() < 1.0 =>
                                {
                                    Some((*p, *s))
                                }
                                _ => None,
                            })
                            .collect();
                        eprintln!("   drawn images of the buttons' size: {same_size:?}");
                        eprintln!("   rect {pos:?} {size:?}; nearest drawn images {:?}", &near[..near.len().min(3)]);
                    }
                }
            }
            eprintln!("{w}x{h}: {missing} of 9 hit rectangles have no drawn image at them");
            assert_eq!(missing, 0, "title buttons are drawn where they are clicked (UI scale {scale}, window {w}x{h})");
            // And they are on the screen (at another UI scale the whole menu was shifted by the capture's half canvas, off it).
            // (The game's UI scales are 0.5 to 1.25: at more the panel is taller than the canvas.)
            assert!(
                scale > 1.25 || hit_rects.iter().all(|(pos, size)| pos.x >= 0.0 && pos.x + size.x <= w && pos.y >= 0.0),
                "title buttons on screen (UI scale {scale}, window {w}x{h}): {hit_rects:?}"
            );
        }
    }

    /// Probe: the title picture with the click rectangles drawn over it (`RUSTIBITES_PROBE_OUT` = a folder to write the PNGs to).
    #[test]
    fn the_title_picture_with_its_click_rectangles() {
        let Ok(out) = std::env::var("RUSTIBITES_PROBE_OUT") else {
            return;
        };
        let Ok(Some(runtime)) = RuntimeUi::discover() else {
            return;
        };
        let Ok(Some(ui)) = UiAssets::discover() else {
            return;
        };
        let Ok(mut fonts) = crate::ui::FontSet::load(&ui) else {
            return;
        };
        for (scale, w, h) in [(1.0_f32, 1920_u32, 1080_u32), (1.5, 1920, 1080), (0.75, 1920, 1080), (1.0, 2560, 1080)] {
            crate::ui::set_ui_scale_settings(true, 1.0, scale);
            let sdf = fonts.sdf();
            let frame = runtime.build_menu_frame(false, &ui, w as f32, h as f32, sdf).expect("frame");
            let rects = runtime.menu_button_rects(w as f32, h as f32);
            let overlay = crate::menu::menu_overlay(&ui, &frame, &mut fonts);
            let mut image = crate::render::render_ui_only(w, h, 1, Some(overlay));
            for (pos, size) in &rects {
                for x in (pos.x.max(0.0) as u32)..((pos.x + size.x).max(0.0) as u32).min(w) {
                    for y in [pos.y, pos.y + size.y] {
                        if y >= 0.0 && (y as u32) < h {
                            let at = 4 * (y as usize * w as usize + x as usize);
                            image.rgba[at..at + 4].copy_from_slice(&[255, 0, 0, 255]);
                        }
                    }
                }
            }
            let _ = image.write_png(format!("{out}/title_{scale}_{w}x{h}.png"));
            eprintln!("{scale} {w}x{h}: rects {:?}", rects.iter().take(2).collect::<Vec<_>>());
        }
        crate::ui::set_ui_scale_settings(true, 1.0, 1.0);
    }

    /// Probe: what one rebuild of the title menu's picture costs at a window's size (it runs on the window's thread at every hover change).
    #[test]
    fn the_title_menu_picture_costs_this_much_at_each_size() {
        let Ok(Some(runtime)) = RuntimeUi::discover() else {
            return;
        };
        let Ok(Some(ui)) = UiAssets::discover() else {
            return;
        };
        let Ok(mut fonts) = crate::ui::FontSet::load(&ui) else {
            return;
        };
        for (w, h) in [(1280_u32, 720_u32), (1920, 1080), (2560, 1440)] {
            let mut last = (0.0, 0.0);
            for _ in 0..4 {
                let started = std::time::Instant::now();
                let sdf = fonts.sdf();
                let frame = runtime.build_menu_frame(false, &ui, w as f32, h as f32, sdf).expect("frame");
                let built = started.elapsed().as_secs_f64() * 1000.0;
                let overlay = crate::menu::menu_overlay(&ui, &frame, &mut fonts);
                let _ = crate::render::render_ui_only(w, h, 1, Some(overlay));
                last = (built, started.elapsed().as_secs_f64() * 1000.0);
            }
            eprintln!("{w}x{h}: frame {:.0} ms, picture {:.0} ms in all", last.0, last.1);
        }
    }
}
