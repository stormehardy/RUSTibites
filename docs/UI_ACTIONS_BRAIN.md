# UI actions — brain, genome, lineage, species

Status: **functional/UI parity survey**, 2026-11-02. Scope: game 0.6.3.1
`Here/UIScripts*` panels that expose brain, genome, lineage, or species data,
mapped to `bibites-core` APIs. This file is the contract for the viewer; it does
**not** authorize viewer UI edits from brain-dev. Source files are under
`Here/`; read-only/viewer work is owned by view-dev, live-entity queries by
body-dev/world-dev.

## Existing read-only core surface

| Need | Core API |
|---|---|
| Brain nodes (array order, type, innovation, bias, runtime value/last in/last out) | `brain::Brain::to_node_records`, `node_state`, `node_value`, `node_type`, `node_innovation`, `node_desc` |
| Brain synapses (template order, endpoints, weight, enabled, innovation) | `Brain::to_synapse_records` |
| Inputs/outputs by enum/index and names | `brain::{Input, Output}`; `Brain::{input_value, output_value, output_bias}` |
| Node-type editor metadata | `NodeType::{has_bias, uses_bias_differently, is_non_linear, name}` |
| Live output snapshot | `Brain::output_values_copy` / `output_value` |
| Brain comparison for highlighting | **new** `Brain::differences` → `BrainDiff` (connection/activation deltas, disjoint/toggled innovations, `total_distance`) |
| Genome values | `genome::Genome::{gene, values, growth_at_mature, ...}` |
| Gene display/edit metadata | `genome::{Gene, GENE_DEFS, GeneDef}` (`name`, `display_name`, `min`, `max`, `default`, `units`, `purely_relative`, `span`) |
| Species record | `species::SpeciesRegistry::{get, records, active_species}` → `SpeciesRecord` |
| Species lineage | `SpeciesRecord::{parent, root, children}`; `SpeciesRegistry::{parent_of, ancestors, roots, descendants_of, lineage_edges}` |
| Species stats/history | `SpeciesRecord::stats` (`count`, `energy`, `n_points_present`, `total_index_of_apparition`, bounded `history`) |
| Extinction filter | **new** `SpeciesRecord::is_extinct`, `SpeciesRegistry::extinct_species` |
| Species template comparison | `TemplateSnapshot::{gene_distance, brain_distance, distance}`, `SpeciesRegistry::distance_to` |
| Species template data | `SpeciesRecord.template.{genome, brain}` |
| Species favorite/prune | `SpeciesRegistry::set_favorite`, `prune_from_tree`, `species_tree_cleanup` |
| Live entities | `World::bibites` iterator; records carry `GenomeMeta` (`parent1`, `parent2`, `species`, generation) and public body/brain/genome state |

## BrainPanel / SpeciesBrainPanel (read-only)

| UI action | Game source | Core mapping | State |
|---|---|---|---|
| Open panel for selected bibite/species | `BrainPanel.FillPanel`, `SpeciesBrainPanel` | `BibiteRecord.brain`, `SpeciesRecord.template.brain` | available |
| Draw nodes and synapses | `BrainPanelCommon.FillBrainPanel` | `to_node_records` + `to_synapse_records`; draw in template order | available |
| Show node values/names | `UINeuron.SetValue/SetDesc` | `node_state.value`, `node_desc`; `UserSettings.ShowNodeValues` is viewer preference | available |
| Show node/synapse counts | `BrainPanelCommon` | `node_count`, `synapse_count` | available |
| Species name and distance | `BrainPanelCommon`, `BrainPanel` | `SpeciesRecord::name`, `TemplateSnapshot::brain_distance`, `SpeciesRegistry::used_span` | available |
| Brain-difference highlighting | `BrainPanel.ShowDifferences` / `Species.BrainDifferencesToIndividual` | **new** `Brain::differences` → `BrainDiff` (`connection_deltas[].template_index`, `activation_deltas[].template_index`, `disjoint_*`, `toggled`) | available read-only |
| Node anchors/layout | `species.template.nodeAnchors` | io `TemplateMeta::{node_anchor,set_node_anchor,remove_node_anchor,clear_node_anchors}` and `LoadedGenome` wrappers; core `SpeciesRecord.template` still has no anchor map | editor/io available for templates |
| Graph sorting/groups/used indices | `BrainSorter`, `NodeGroup`, `BrainPanelCommon` | view-side graph algorithms; core deliberately has no layout/group API | viewer-owned |

## BibiteBrainEditor / ExpandedBrainPanel

| UI action | Game source | Core mapping | State |
|---|---|---|---|
| Load editor graph nodes/synapses/anchors | `BibiteBrainEditor.FillEditor` | records/graph view + `TemplateMeta.node_anchor` keyed by innovation | available |
| Add node/connection, delete node | `CreateNewConnectionFromToNode`, `DeleteNode` | `Brain::{add_hidden_node, remove_hidden_node, add_synapse_connection, remove_synapse_at}` | editing available |
| Edit node label/function/bias/input activation | `NodeEditPanel.EditLabel/EditFunction/EditBias/EditAct` | read: `node_desc`, `node_type`, `node_state`, `NodeType::*`; write: `set_hidden_node_label`, `set_hidden_node_type`, `set_node_bias_at`, `set_output_bias` | editing available (base labels/types fixed) |
| Edit synapse weight/enabled | `ConnectionEditor.EditWeight/EditEnabled` | `set_synapse_weight`, `set_synapse_enabled`; read from `to_synapse_records` | editing available |
| Show/hide disabled synapses | `DisableSynapse` → `UISynapse.ToggleShowDisabled` | viewer rendering choice over `enabled` records | available |
| Rebuild/reset node effects | `ReSortBrain`/`ExecuteNode` | `Brain::rebuild`, `initialize_runtime_states`, `validate_editor`; live graph should not mutate simulation from viewer | view/editor model |
| Force graph/start/stop/BFS/trim branches | `ExpandedBrainPanel` | viewer layout algorithms over node/synapse records; no core physics | viewer-owned |
| Save anchors / template save | `SaveAnchors`, `PrepareForTemplateSave` | `LoadedGenome::{set_node_anchor,node_anchor}` / `TemplateMeta.node_anchors`, then `to_json` | editing/format available for templates |

## Genome/gene panels

| UI action | Game source | Core mapping | State |
|---|---|---|---|
| Display 35 genes with name/unit/value | `GenesPanel`, `GeneLineInfo` | `Genome::values`, `GENE_DEFS`, `Gene::{display_name,units,range,default_value}` | available |
| Species gene distance and per-gene contribution | `GeneDistanceToIndividual`, `GeneLineInfo` | `TemplateSnapshot::gene_distance`, `GeneDef::max/span` | available; UI computes contribution |
| Gene tooltip/helper text | `GeneSetting.HelperText` | **missing**: `GeneDef` has display/unit/range but no description/helper text | minor read-only metadata gap |
| Edit gene values in editors | `GeneSetting`, `RandomizeGenes` | `Genome::{set_gene,randomized,randomize_color,mutate_gene}` + `GeneDef` bounds | editing available |
| Show species/parent gene differences | `GenesPanel`, `GeneLineInfo` | template genome + `GeneDef::max`; parent is `SpeciesRecord.parent` | available |

## Lineage panels

| UI action | Game source | Core mapping | State |
|---|---|---|---|
| List/iterate recorded species | `LineageMeshPanel.GenerateMesh`, `LineageTreePanel.BuildTree` | `SpeciesRegistry::records` / `lineage_edges` | available |
| Parent/root traversal | lineage select/parent/root buttons | `SpeciesRecord.{parent,root}`, `SpeciesRegistry::{parent_of, ancestors, roots}` | available |
| Child/descendant traversal | mesh/tree columns | `SpeciesRecord.children`, new `descendants_of`/`descendant_species_count` | available |
| Hide extinct / show favourites | `hideExtinctLineages`, `showLineagesOfFavorite` | `is_extinct` / `extinct_species`, `favorite`, `active_species` | predicate available; exact game extinction vertical index missing |
| Species presence time / rung placement | `LineageMesh`, `LineageTreePanel`, `SpeciesRungData` | only `total_index_of_apparition` + bounded `history`; **no `totalIndexOfDisappearance`, per-rung `nPointsPresent`/energy arrays** | **read-only data gap** (see below) |
| Species energy/count bars | `LineageMesh`/`LineageSpeciesNode` | `stats.{count,energy,history}` | partial (latest/bounded history) |
| Click/focus/select species | `SpeciesPanel.SelectAndFocusSpecies`, `LineageMeshPanel`, `LineageTreePanel` | `SpeciesId` is stable; selection is viewer state | available |
| Compare dropdown | `LineagePanelSettingsManager`, `SpeciesTabManager` | `SpeciesRegistry::records`/get by id | available |
| Prune species | `SpeciesInfoPanel.PruneSpecies` | `SpeciesRegistry::prune_from_tree` exists; exact UI enablement uses disappearance index | editing API available, history gap |
| Ancestral lineage scroll/view | `AncestralLineageScroll`, lineage nodes | `ancestors` / `lineage_edges` | available |

### Important lineage data gap
Game `speciesData` stores a compressed per-time presence array with
`totalIndexOfApparition`, `totalIndexOfDisappearance`, per-scale `nPresent` and
per-rung energy/count values. Core keeps only `total_index_of_apparition`,
bounded `SpeciesHistoryPoint` (512), and no disappearance index. Therefore:
- exact extinct-lineage filtering by disappearance time,
- vertical rung placement at extinction,
- branch widths/energies over all recorded rungs,

cannot be reproduced exactly from current core data; a viewer can approximate
from `history`/`count`/`is_extinct`. Adding the compressed species-data timeline
is a **data-model/io** task (fmt-dev/world-dev), not a brain evaluator change.

## Species tabs

| UI/action | Game source | Core mapping | State |
|---|---|---|---|
| Species info: name, creation time, generation, member count, descendant count | `SpeciesInfoPanel` | `SpeciesRecord` fields + `descendant_species_count`; `time_creation` | available |
| Rename/description edit | `SpeciesInfoPanel` name editor | fields are public; no validation/normalization helper | editing gap (UI/io) |
| Favorite/unfavorite | `SpeciesInfoPanel` | `set_favorite` | available |
| Prune species | `PruneSpecies` | `prune_from_tree`; enablement history gap | available with gap |
| Save template / place species | `SaveController.SaveTemplate`, `BibitePlacer` | `TemplateSnapshot`→`SavedTemplate`/io save; editor anchors missing | editing/format gap |
| Select random bibite of species | `UserControl.SelectRandomBibiteOfSpecies` | iterate `World::bibites`, filter `meta.species`; no convenience reverse index | available, body/view helper desirable |
| Compare parent/root | `SpeciesTabManager.SelectParent/SelectRoot` | parent/root fields | available |
| Species genes/brain tabs | `SpeciesGenesPanel`, `SpeciesBrainPanel` | template genome/brain, distances, new `Brain::differences` | available |
| Species distribution charts | `SpeciesDistributionPanel` | **works** (2026-11-05): the species' and zones' multi-resolution logs are ported and read from `data.bin`; see PARITY_REMAINING "Species Position tab" | none |

## Follow/ancestry and live-entity actions

| Action | Game source | Core mapping | State |
|---|---|---|---|
| Follow selected bibite/camera | viewer selection | `BibiteId` stable; position/velocity in `BibiteRecord` | available |
| Open lineage of current species | `GenesPanel.OpenLineagePanelAndSelectSpecies`, panels | `GenomeMeta.species` → `SpeciesRegistry::get`/ancestors | available |
| Select a bibite’s parent/child | ancestry/selection | `GenomeMeta.parent1/parent2`, `children` are runtime ids; parent may be dead | available ids; viewer decides follow |
| Find bibites in a species | species select action | iterate `World::bibites` + `GenomeMeta.species` | available; reverse-index helper optional |
| Select random species member | `SelectRandomBibiteOfSpecies` | same iteration | available; body/view helper optional |

## Newly implemented read-only APIs (this task)

- `Brain::differences(&self, other) -> BrainDiff`
  - `BrainConnectionDelta { template_index, innovation, delta, distance }`
  - `BrainActivationDelta { template_index, innovation, delta, distance }`
  - lists `disjoint_template`, `disjoint_individual`, `toggled`
  - `BrainDiff::total_distance()` matches `Brain::distance` speciation metric
  - Regression: `differences_expose_template_indices_and_match_distance`.
- `NodeType::is_non_linear()` (`NEATBrain.TypeIsNonLinear`).
- `SpeciesRecord::is_extinct()`.
- `SpeciesRegistry::{descendants_of, roots, lineage_edges, extinct_species}`.
  - Regression: `lineage_traversal_and_extinction_helpers`.

### Viewer-friendly snapshots (in-world panels)

These are plain owned structs/iterators intended for view-dev; they do not
borrow the registry/brain while the simulation continues.

- `Brain::graph_view() -> BrainGraphView`
  - `nodes: Vec<BrainNodeView { index, node_type, innovation, label, base_activation, value, last_input, last_output }>`
  - `synapses: Vec<BrainSynapseView { index, innovation, node_in, node_out, weight, enabled }>`
  - `outputs: [f32; N_OUTPUTS]` (input values are the caller's current
    `BrainInputs`; the brain does not store them)
  - Regression: `graph_view_exposes_nodes_synapses_and_outputs`.
- `SpeciesRegistry::species_list() -> Vec<SpeciesListEntry>` — id, names,
  description, parent/root/children, count/energy, favourite/active/extinct,
  creation time/generation and retained history-point count.
- `SpeciesRegistry::lineage_tree() -> LineageTree`
  - `roots` and `edges: Vec<LineageEdge { parent, child }>` for graph rendering.
  - `nodes: Vec<LineageNode>` in record order with `depth` (root = 0),
    `order`, `child_count`, `member_count`, `energy`,
    `subtree_species_count`, `subtree_member_count`,
    `generation_of_first_specimen`, `favorite`, `active`, `extinct`, and
    `history` distribution samples.
  - `max_depth` for layout bounds.
- `SpeciesRegistry::selected_lineage(id) -> Option<LineageSelection>` — selected,
  parent/root, direct children, parent-upward ancestors, depth-first
  descendants and root→selected navigation path; useful for the AncestralLineage
  parent/root/descendant controls.
- Regression: `species_and_lineage_views_are_self_contained`.

### Lineage layout/distribution: exact vs approximate

| Data | Viewer API | Fidelity |
|---|---|---|
| Roots, parent→child edges | `LineageTree.{roots,edges}` | exact registry topology |
| Generation rank/order | `LineageNode.{depth,order}`, `max_depth` | exact parent-chain depth; **not** the game's apparition-index rung placement |
| Species count / direct children | `LineageNode.{subtree_species_count,child_count}` | exact over current registry records |
| Individual count/energy | `LineageNode.{member_count,energy}` | exact current core aggregate (live + incubating); game per-rung historical counts are not stored |
| Subtree individual count | `LineageNode.subtree_member_count` | exact over current core counts, recursively summed |
| Distribution samples | `LineageNode.history` (`SpeciesHistoryPoint {time,count,energy}`) | **approximate**: retained bounded history (60 s cadence, max `MAX_HISTORY_POINTS`); game's compressed per-rung/zone distribution arrays and disappearance index are not modelled |
| Distribution window / absolute rung indices | `SpeciesRegistry::history_window(id)` → `SpeciesHistoryWindow` (`points`, `start_index`, `end_index`, `n_points_present`, `truncated`, cadence, active/extinct) | exact **within the retained window**; older samples are evicted and cannot be reconstructed |
| Root/parent/ancestors/descendants selection | `selected_lineage` | exact parent/child traversal over current records |

**History/rung assessment:** core logs one count/energy point per species every
`SPECIES_LOG_PERIOD_SECONDS` and retains at most `MAX_HISTORY_POINTS` (512);
there is no eviction archive. Therefore exact per-species rung distribution
**beyond the retained window is not feasible from existing records** without a
model/io change to persist a compressed archive (no game capture is involved).
Within the window, `history_window` exposes the points plus absolute log
indices (`total_index_of_apparition`-derived) so a lineage view can place them on
a consistent x axis; `truncated` marks the missing older span. Game
`speciesData` rung arrays, per-zone splits, and `totalIndexOfDisappearance`
remain unavailable; use `LineageNode.{history_start_index,history_truncated}`
for layout and label the chart as retained/approximate.


### Editor construction/usage snippet (view-dev)

```rust
use bibites_core::brain::Brain;
use bibites_core::genome::{Gene, Genome, MutationFactor};
use bibites_core::rng::Rng;
use bibites_core::settings::SimSettings;

// New library entry: starter or randomized genome + starter brain.
let mut rng = Rng::from_seed(42);
let settings = SimSettings::game_defaults();             // or the live world settings
let factor = MutationFactor::default();                  // or radiation/event pressure
let mut genome = Genome::randomized(&mut rng);           // game RandomGenes post-processing
genome.set_gene(Gene::LayTime, 20.0);                    // clamped to GeneDef range
let _ = genome.mutate_gene(Gene::SizeRatio, &mut rng, &settings, &factor);

// Gene editor row metadata:
let (min, max) = Gene::LayTime.range();
let label = Gene::LayTime.display_name();                // "Lay Time"
let units = Gene::LayTime.units();                       // "s"
let default = Gene::LayTime.default_value();

let mut brain = Brain::starter();
let hidden = brain.add_hidden_node(
    bibites_core::brain::NodeType::TanH, Some("Editor hidden"),
).expect("hidden node");
brain.add_synapse_connection(0, hidden, 1.0, true);       // input -> hidden
brain.add_synapse_connection(hidden, 33, 1.0, true);      // hidden -> output
brain.set_synapse_weight(0, 2.0);
brain.set_synapse_enabled(0, true);
brain.set_node_bias_at(hidden, 0.5);
brain.set_hidden_node_label(hidden, "Renamed");
let validation = brain.validate_editor();                 // floating nodes are OK
assert!(validation.is_valid());

// io template object (also available via core `TemplateSource::starter`):
let mut template = bibites_io::LoadedGenome::starter_template("Library Bibite");
template.genome = genome;
template.brain = brain;
let innovation = template.brain.node_innovation(hidden).unwrap();
template.set_node_anchor(innovation, -120.0, 80.0);       // nodeAnchors map
assert_eq!(template.node_anchor(innovation), Some((-120.0, 80.0)));
let json = template.to_json()?;                           // .bb8template
```

Gene-editor rows use `genome::ALL_GENES`, `Gene::def()` and `GeneDef` fields
(`display_name`, `min`, `max`, `units`, `purely_relative`), or the convenience
`Gene::{display_name, units, range, default_value, span}`. Brain node labels
come from `BrainNodeView.label` / `Brain::node_desc`; hidden labels are writable
via `set_hidden_node_label`, base input/output labels are fixed. Node anchors are
**not** in `Brain`: use `TemplateMeta`/`LoadedGenome`
`{node_anchor,set_node_anchor,remove_node_anchor,clear_node_anchors}` (keyed by
node innovation; `AnchorJson::{new,position}` for raw map access) so positions
round-trip through `.bb8template`.


| Gap | Kind | Owner |
|---|---|---|
| Node anchors on `SpeciesRecord.template` | core species snapshot still has no anchor map; io `TemplateMeta`/`LoadedGenome` supports template-editor anchors | fmt/view |
| Exact species disappearance index + per-rung presence/energy timeline | species data model/io; current bounded history insufficient | fmt/world + brain-dev data model |
| Species distribution chart data (zone/history) | **partial**: `LineageNode.history` exposes bounded count/energy samples; game compressed per-rung/zone arrays and disappearance index still missing | fmt/world + brain-dev data model |
| Gene helper/description text | display metadata on `GeneDef` (name/unit/range done; helper text still missing) | brain-dev minor |
| Node input/output activation display ranges | editor metadata constants missing | view/UI |
| Reverse index species→live bibites / random member | convenience query | body/world |
| Template save/place and species rename validation | editor/format path | view/fmt |

No viewer UI edits were made. View-dev can consume the read-only APIs above;
body/world can add member lookup helpers without changing simulation behavior.
