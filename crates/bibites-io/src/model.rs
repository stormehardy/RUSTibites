//! Serde JSON models for `.bb8` genome/entity files and `.bb8template` files.
//!
//! These models mirror the game's JSON exactly (`docs/spec/04-formats-settings.md`
//! section 4) and preserve unknown fields in `extra` maps so a load -> save
//! round-trip does not strip data written by another game version.
//!
//! The per-entity sections (`transform`, `rb2d`, `body`, `clock`, `egg`) are
//! modelled with their known keys in game order plus an `extra` map. `bibites-io`
//! I1 only needs genes + brain; later slices (save bundles) will type more of
//! the entity state and share these models.
//!
//! ## Field order
//!
//! Serde serializes struct fields in declaration order. The writer order here
//! matches the game's writer for the templates (bible 4.5) and the reduced
//! genome-only `.bb8` shape. Order is not significant to the game's readers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A JSON object of fields this crate does not model (preserved verbatim).
pub type Extra = Map<String, Value>;

/// One brain node as it appears in `.bb8` and `.bb8template` JSON.
///
/// Template files only carry `Type`, `Index`, `Inov`, `Desc`, `baseActivation`;
/// live entities also carry `TypeName`, `Value`, `LastInput`, `LastOutput` and
/// `archetype`. Optional fields let one model read both.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NodeJson {
    /// Activation type integer (`NodeType` value).
    #[serde(rename = "Type", default, skip_serializing_if = "Option::is_none")]
    pub node_type: Option<u8>,
    /// Array position (informational; constructors use slice order).
    #[serde(rename = "Index", default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    /// Innovation number.
    #[serde(rename = "Inov", default, skip_serializing_if = "Option::is_none")]
    pub innovation: Option<i64>,
    /// Description / base-node name.
    #[serde(rename = "Desc", default, skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    /// Bias / default activation.
    #[serde(
        rename = "baseActivation",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub base_activation: Option<f32>,
    /// `NodeType.ToString()` mirror (live entities only).
    #[serde(rename = "TypeName", default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// Current output (live entities only).
    #[serde(rename = "Value", default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f32>,
    /// Previous full activation (live entities only).
    #[serde(rename = "LastInput", default, skip_serializing_if = "Option::is_none")]
    pub last_input: Option<f32>,
    /// Previous output (live entities only).
    #[serde(
        rename = "LastOutput",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_output: Option<f32>,
    /// Read-only archetype mirror (live entities only; ignored on load).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archetype: Option<u32>,
    /// Unknown fields, preserved.
    #[serde(flatten)]
    pub extra: Extra,
}

/// One synapse as it appears in `.bb8` and `.bb8template` JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SynapseJson {
    /// Innovation number.
    #[serde(rename = "Inov", default)]
    pub innovation: i64,
    /// Source node array index. Negative values (the game writes -1 for
    /// dead/removed endpoints) are parsed as u32::MAX and the tolerant reader
    /// drops such synapses, mirroring the game's load.
    #[serde(rename = "NodeIn", default, deserialize_with = "deserialize_index")]
    pub node_in: u32,
    /// Destination node array index (see [Self::node_in]).
    #[serde(rename = "NodeOut", default, deserialize_with = "deserialize_index")]
    pub node_out: u32,
    /// Connection weight.
    #[serde(rename = "Weight", default)]
    pub weight: f32,
    /// Whether the connection transmits.
    #[serde(rename = "En", default)]
    pub enabled: bool,
    /// Unknown fields, preserved.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for SynapseJson {
    fn default() -> Self {
        Self {
            innovation: 0,
            node_in: 0,
            node_out: 0,
            weight: 0.0,
            enabled: false,
            extra: Extra::new(),
        }
    }
}

/// Brain object (`brain` in `.bb8`, the top level of a template's brain data).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BrainJson {
    /// Brain readiness flag (live entities only).
    #[serde(rename = "isReady", default, skip_serializing_if = "Option::is_none")]
    pub is_ready: Option<bool>,
    /// Nodes, in array-index order.
    #[serde(rename = "Nodes", default)]
    pub nodes: Vec<NodeJson>,
    /// Synapses.
    #[serde(rename = "Synapses", default)]
    pub synapses: Vec<SynapseJson>,
    /// Unknown fields, preserved.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Gene values as they appear in a file: a name -> value object (current) or a
/// flat float array (pre-0.6a1 templates).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GenesRepr {
    /// Current shape: gene name -> value (plus legacy Strength/Defence keys).
    Object(Map<String, Value>),
    /// Legacy shape: flat array (34 values before 0.6a1, 35 later).
    Array(Vec<f32>),
}

/// The `genes` object of a live bibite file (`.bb8` / save bundle).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Bb8Genes {
    /// Lineage tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Species database id.
    #[serde(rename = "speciesID", default, skip_serializing_if = "Option::is_none")]
    pub species_id: Option<i64>,
    /// Readiness flag.
    #[serde(rename = "isReady", default, skip_serializing_if = "Option::is_none")]
    pub is_ready: Option<bool>,
    /// Generation number.
    #[serde(rename = "gen", default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u32>,
    /// Speciation outlier flag.
    #[serde(rename = "isMutant", default, skip_serializing_if = "Option::is_none")]
    pub is_mutant: Option<bool>,
    /// Radiation easter-egg flag.
    #[serde(rename = "isRad", default, skip_serializing_if = "Option::is_none")]
    pub is_rad: Option<bool>,
    /// Rainbow easter-egg flag.
    #[serde(rename = "isRain", default, skip_serializing_if = "Option::is_none")]
    pub is_rain: Option<bool>,
    /// Reaper easter-egg flag.
    #[serde(rename = "isKiller", default, skip_serializing_if = "Option::is_none")]
    pub is_killer: Option<bool>,
    /// Pellet-source easter-egg flag.
    #[serde(rename = "isSource", default, skip_serializing_if = "Option::is_none")]
    pub is_source: Option<bool>,
    /// First parent runtime id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent1: Option<i32>,
    /// Second parent runtime id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent2: Option<i32>,
    /// The 35 gene values (nested under `genes` in the game's shape).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genes: Option<GenesRepr>,
    /// Unknown fields, preserved.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Reads `[x, y]` where a number may be `null`: a position that was NaN when a build wrote the save (it wrote NaN as `null`, which
/// no reader of a plain `f32` takes back) reads as NaN, and the loader leaves such a bibite out.
fn position_or_nan<'de, D>(deserializer: D) -> std::result::Result<Option<[f32; 2]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Option<[Option<f32>; 2]> = Option::deserialize(deserializer)?;
    Ok(raw.map(|[x, y]| [x.unwrap_or(f32::NAN), y.unwrap_or(f32::NAN)]))
}

/// Serialized transform (`transform`; `SerializedTransform` in the game).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TransformJson {
    /// `[x, y]` local position (`null` reads as NaN).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "position_or_nan"
    )]
    pub position: Option<[f32; 2]>,
    /// Z rotation in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f32>,
    /// Uniform local scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    /// Unknown fields, preserved.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Serialized rigid body (`rb2d`; `SerializationHelper.SerializeRigidBody2D`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Rb2dJson {
    /// Position x.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub px: Option<f32>,
    /// Position y.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub py: Option<f32>,
    /// Velocity x.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vx: Option<f32>,
    /// Velocity y.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vy: Option<f32>,
    /// Rotation in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<f32>,
    /// Unknown fields, preserved.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Body section of a live bibite file (subset needed for preservation).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BodyJson {
    /// Health points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<f32>,
    /// Energy points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy: Option<f32>,
    /// Body area state (`d2Size`), effectively required by the game loader.
    #[serde(rename = "d2Size", default, skip_serializing_if = "Option::is_none")]
    pub d2_size: Option<f32>,
    /// Entity id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    /// Corpse flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dead: Option<bool>,
    /// Dying flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dying: Option<bool>,
    /// Unknown fields, preserved (mouth, stomach, phero, eggLayer, ...).
    #[serde(flatten)]
    pub extra: Extra,
}

/// Internal clock section of a live bibite file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClockJson {
    /// Internal clock phase (0/1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tic: Option<i32>,
    /// Seconds into the current clock period.
    #[serde(
        rename = "ticProgress",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub tic_progress: Option<f32>,
    /// Seconds alive.
    #[serde(rename = "timeAlive", default, skip_serializing_if = "Option::is_none")]
    pub time_alive: Option<f32>,
    /// Seconds since the last clock reset.
    #[serde(
        rename = "chronoTime",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub chrono_time: Option<f32>,
    /// Unknown fields, preserved.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Egg section of an egg file (`egg`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EggJson {
    /// Hatch progress in seconds.
    #[serde(
        rename = "hatchProgress",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub hatch_progress: Option<f32>,
    /// Stored energy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy: Option<f32>,
    /// Entity id (observed 0 in real saved eggs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    /// Unknown fields, preserved.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A `.bb8` file: either a living bibite/egg snapshot or a genome-only export.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bb8File {
    /// Transform section (absent in genome-only exports).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<TransformJson>,
    /// Rigid body section (absent in genome-only exports).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rb2d: Option<Rb2dJson>,
    /// Genome data.
    pub genes: Bb8Genes,
    /// Body section (living bibites only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<BodyJson>,
    /// Clock section (living bibites only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<ClockJson>,
    /// Egg section (eggs only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egg: Option<EggJson>,
    /// Brain data.
    pub brain: BrainJson,
    /// Game version string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// User description (exports only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    /// Unknown top-level sections, preserved.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Node anchor: a brain-editor position for one innovation number.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AnchorJson {
    /// X position.
    #[serde(default)]
    pub x: f32,
    /// Y position.
    #[serde(default)]
    pub y: f32,
    /// Unknown fields, preserved.
    #[serde(flatten)]
    pub extra: Extra,
}

impl AnchorJson {
    /// Creates an anchor at `(x, y)` with no preserved unknown fields.
    #[must_use]
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            extra: Extra::new(),
        }
    }

    /// Returns the `(x, y)` position.
    #[must_use]
    pub fn position(&self) -> (f32, f32) {
        (self.x, self.y)
    }
}

/// A `.bb8template` file (genome + brain template).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TemplateFile {
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Species display name (recomputed on load by the game).
    #[serde(
        rename = "speciesName",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub species_name: Option<String>,
    /// Free-form description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Generation of the source specimen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u32>,
    /// Game version that wrote the file.
    pub version: String,
    /// Shipped-template flag.
    #[serde(rename = "isOfficial", default, skip_serializing_if = "is_false")]
    pub is_official: bool,
    /// Nodes, in array-index order.
    #[serde(default)]
    pub nodes: Vec<NodeJson>,
    /// Synapses.
    #[serde(default)]
    pub synapses: Vec<SynapseJson>,
    /// Gene values (object for current, array before 0.6a1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genes: Option<GenesRepr>,
    /// Brain-editor node anchors keyed by innovation number.
    #[serde(
        rename = "nodeAnchors",
        default,
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub node_anchors: BTreeMap<i64, AnchorJson>,
    /// Unknown fields, preserved.
    #[serde(flatten)]
    pub extra: Extra,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Deserialises a node index allowing the game's -1 sentinel. Negative values
/// become [u32::MAX] so the reader can distinguish them from valid indexes.
fn deserialize_index<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let value = i64::deserialize(deserializer)?;
    if value < 0 {
        Ok(u32::MAX)
    } else {
        u32::try_from(value)
            .map_err(|_| serde::de::Error::custom(format!("node index {value} exceeds u32")))
    }
}
