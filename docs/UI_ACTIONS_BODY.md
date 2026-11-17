# Bibite UI action/API map

Survey of `Here/UIScripts` and `Here/ManagementScripts` for the bibite-facing UI, mapped to
core/body APIs. Rust does not own UI code; this document is the contract for view-dev and records
which core-facing actions are already available versus missing.

## Template library / default bibites

| UI feature | Here source | Rust core/API | Status |
|---|---|---|---|
| Template library list (built-in/saved tabs), preview selection | `BibiteTemplateSelectorPanel` (`FillLists`, `ItemClicked`, `OpenTab`, previewer) | `bibites_io` template loading + `World::set_templates(Vec<TemplateSource>)`; `TemplateSource {name,genome,meta,brain}` | file/load present; list model/preview selection is view-side |
| Choose template for spawn/editor | `BibiteTemplateSelectorPanel.OpenWithCallBacks`, `OpenForBibiteEditor`, `OpenEditorForNewBibite` | `TemplateSource` + `World::spawn_template_at` / `spawn_bibite_from_template`; editor UI missing | partial |
| Save/share/delete/workshop template | `BibiteTemplateSelectorPanel` workshop/delete actions | `bibites_io::save_template_source` + save-browser facade (`3f7a593`/`bdd6125`); share/workshop still missing | save present, library/share view-owned |
| Default bibite list, add/remove/delete-all | `DefaultBibitesPanel`, `BibiteSettingsHandle` | `ScenarioData.bibite_spawns`/`SpawnRow`; `World::set_templates`; no add/remove default-bibite API | missing |
| Challenge/default mode restrictions | `ChallengeManager.isChallenge` checks across panels | no challenge session layer | missing (scenario/session) |

## Spawn / place bibite

| UI feature | Here source | Rust core/API | Status |
|---|---|---|---|
| Place selected template at cursor, pause while placing | `BibitePlacer.PlaceBibitesFromTemplate` | `World::spawn_template_at(&TemplateSource, pos, angle_radians, growth_at_spawn, tagging, tag)` **added** | present |
| Growth at spawn (Egg/Baby/Adult/Elder) | `WorldObjectsSpawner.SpawnBibiteFromTemplate` `targetGrowth` | `GrowthAtSpawn`; `spawn_template_at` returns `TemplateSpawn::Egg`/`Bibite`; existing `spawn_bibite_from_template` returns live only | present for explicit API |
| Tagging (none/species/random/custom) | `WorldObjectsSpawner` `taggingChoice`, `customTag` | `Tagging`; new API supports `NoTagging`/`CustomTagging`; species/random tagging missing | partial |
| Randomize genes (No/NormalMutations/Color/AllGenes) | `WorldObjectsSpawner` `randomizeGenes` | no public in-place `Genome::randomize/mutate`; `Genome::set_gene`, `Gene::def`, `Genome::child` exist | **missing** (genome/brain API) |
| Angle override / random angle | `SpawnBibiteFromTemplate(angle)` | `spawn_template_at` `angle_radians: Option<f32>` **added** | present |
| Species registry/species alignment for placed template | `GlobalLineageManager.TemplateAlreadyHasSpecies`, `AlignInnovations...` | `TemplateSource` + `World` species assign on spawn; template innovation alignment missing | partial (brain/species) |

## Stats panel

| UI field/action | Here source | Rust core/API | Status |
|---|---|---|---|
| Generation, body length, size, mass, speed, time alive, travelled, eggs laid, alive children, bites, damage dealt/received, species tag, species/parents/trait flags, dead/corpse | `BibiteStatsPanel.UpdateBibiteStats`/traits/parents | `BibiteStatsView` via `BibiteRecord::stats_view` or `World::bibite_stats` **added** | present |
| Egg generation/size/mass/energy/progress/free energy | `BibiteStatsPanel.UpdateEggStats` | `EggStatsView` via `EggRecord::stats_view` or `World::egg_stats` **added** | present |
| Edit species tag | `EditSpeciesTag`, `UpdateSpeciesTag` | `World::set_bibite_species_tag` / `set_egg_species_tag`; `bibites_with_tag`, `eggs_with_tag`, `species_tags`, `tag_summaries` (`TagSummary{count/energy}`) **added** | present (scan-based, no persistent index) |
| Kill / explode corpse | `KillCurrentBibite` | `BibiteStore::kill(id, cause)`, `kill_default(id)` | present |
| Revive corpse | `ReviveCorpse` | `World::revive_bibite` restores full health/energy/corpse flags **added** (game coroutine animated over 0.1s) | present |
| Lay egg from current bibite | `LayEggFromCurrentBibite` | `World::force_lay_eggs` lays all completed womb eggs + registers hashes/events **added** | present |
| Save bibite / egg | `SaveCurrentBibite` | `World::bibite_save`/`egg_save` snapshots; `bibites_io::{save_bibite_entity,save_egg_entity}` write standalone `.bb8` files (fmt-dev `3f7a593`) | present |
| Edit bibite/template preview | `EditBibite` | `World::bibite_template_source` / `egg_template_source` return `TemplateSource` for editor; UI editor view-side **added** | core data present |
| Select parent / random child | `SelectParent`, `SelectRandomChild` | `BibiteStatsView.parent1/parent2/species`; `World::live_bibite_ids`/selection helpers **added** | present (random-child view can pick) |

## Genes panel

| UI feature | Here source | Rust core/API | Status |
|---|---|---|---|
| Show all genes and difference from species | `GenesPanel`, `GeneLineInfo` | `Genome::gene(Gene)`, `Gene::def`, `ALL_GENES`; `Brain::differences`/`BrainDiff`; `SpeciesRegistry::{descendants_of,roots,lineage_edges,extinct_species,species_list,lineage_tree,selected_lineage}`/`SpeciesRecord::is_extinct` (brain-dev `e249cc9`/`6b6eb97`) | present data; per-gene UI diff aggregate still view-side |
| Gene editing in template editor | `BibiteEditorPanel` gene sliders | `Genome::set_gene` clamps to `GeneDef`; `Genome::cap` | present basic; template mutation/persist view-side |
| Randomize color/all genes | `BibiteGenes.RandomColor/RandomGenes` | no public randomize API | **missing** |

## Biology panel

| UI group | Here source | Rust core/API | Status |
|---|---|---|---|
| Health: current/max, energy densities, movement penalty, heal rate | `BiologyPanel` health fields | `body.health`, `params`, `heal_max_rate`, `movement_penalty` public | present |
| Energy: stored/max, rates (digest/move/growth/meta/phero/heal/fat/brain/egg), egg cost | `BiologyPanel` energy fields | stomach/fat/egg/brain derived fields public; `FeedingCounters` cumulative rates; no single per-rate snapshot | partial |
| Growth: progress, size/maturity factors, mature/can-lay | `BiologyPanel` growth fields | `growth` state/genes, `egg_layer.is_mature`, `d2/d1` | present fields, aggregate view missing |
| Age: threshold/factor/strength/metabolism penalty | `BiologyPanel` age fields | `age_hours`, `ageing_factor`, penalties public | present |
| Body/organs: sizes, armor, muscles, fat inflation, masses, stomach/womb/throat/jaw/fat sections | `BiologyPanel` + `Organs` enum | `body.geometry`, `armor`, `propulsion_derived`, `fat`, `stomach`, `egg_layer`, `mouth_derived` public | present; per-organ aggregate view missing |
| Internals visualization/interaction | `BiologyPanel` raycast/internals map | no rendering data API | missing (view) |

## Brain panel / brain editor

| UI feature | Here source | Rust core/API | Status |
|---|---|---|---|
| Show nodes/synapses, values, bias, activation/function, differences vs template | `BrainPanel`, `NodeEditor`, `ConnectionEditor` | `Brain::to_node_records`, `to_synapse_records`, `NodeRecord`, `SynapseRecord`, `output_value`; `Brain::differences`/`BrainDiff`, `Brain::graph_view`, `NodeType::is_non_linear` (brain-dev `e249cc9`/`6b6eb97`) | present data |
| Edit bias/activation/function, weight/enabled | `NodeEditor.EditBias/EditAct/EditFunction`, `ConnectionEditor.EditWeight/EditEnabled` | edit `NodeRecord`/`SynapseRecord` then `Brain::from_records` | present basic |
| Add/remove node/connection, split connection, archetypes, validity | `BibiteBrainEditor.AddNode/DeleteNode/DeleteConnection/SplitConnectionInNewNode`, `RequestAddNode`, `NodeArchetype` | no public add/remove/archetype helpers; raw record edit needed | **missing** (brain-dev) |
| Sort/step/play brain, show values | `SortStepBrain`, `StepBrain`, `OneBrainStep`, `PlayBrain` | `Brain::rebuild`, step APIs internal; no public one-step evaluation helper | **missing** (brain-dev) |
| Template save/prepare lists | `PrepareForTemplateSave`, `RebuildLists` | `Brain::to_node_records/to_synapse_records` + `bibites_io::save_template_source` (`3f7a593`) | present data/persist; list prep view-side |

## Selection / follow / camera

| UI feature | Here source | Rust core/API | Status |
|---|---|---|---|
| Select target, next/previous, random/oldest/highest-gen, nearest | `UserControl.SelectTarget/SelectRandomBibite/SelectOldestBibite/SelectHighGenBibite/SelectClosest*` | `BibiteStore::{live_ids,next_live_bibite,previous_live_bibite,oldest_live_bibite,highest_generation_live_bibite,nearest_live_bibite}` **added**; `World` wrappers **added** | present single-target helpers |
| Multi-select rectangle, species/tag random, remove from selection | `SelectionPanel`, `UserControl.SelectObjectsInRectangle`, multi-selection list | no batch/query mesh selection API; tags/species query limited to record meta | missing (view/world) |
| Follow camera/target on death reselect | `CameraManager.followTarget`, `toRepeatOnDeath` | view-owned; core provides target ids/positions | view-side |
| Lay eggs/kill/delete/tag selection | `SelectionPanel` actions | kill present; tag/delete/revive/lay missing | partial |

## Challenge status

`ChallengeTrackerPanel`/`ChallengeManager` show challenge description, exit conditions, score, stars,
retry/high-score. Rust has simulation stats (`SimStats`) and species registry, but no challenge
session model, scoring metric, star progression, or retry flow. Out of core/body scope; needs a
scenario/session layer (coordinate view-dev/Lead).

## Implemented in this change

- `World::spawn_template_at` + `TemplateSpawn` (explicit angle, Egg/Baby/Adult/Elder, no/custom tag).
- `BibiteStatsView` / `EggStatsView` + `BibiteRecord::stats_view` / `EggRecord::stats_view`; stats fields now include species/parents/trait flags/corpse state and `World::bibite_stats`/`egg_stats`.
- `BibiteStore` selection helpers and `World` wrappers for next/previous/oldest/highest-gen/nearest.
- Inspector buttons: `World::revive_bibite`; `World::force_lay_eggs`; `set_bibite_species_tag`/`set_egg_species_tag` + `bibites_with_tag`/`eggs_with_tag`/`species_tags`/`tag_summaries`; `bibite_save`/`egg_save`/`bibite_template_source`/`egg_template_source`.
- Regressions `ui_template_spawn_stats_and_selection_helpers`, `inspector_actions_tags_force_lay_revive_and_save_views`.

## Missing / follow-ups

1. Genome randomize/mutate API for template spawn modes (AllGenes/NormalMutations/Color, species/random tagging).
2. Indexed tag manager (current tag APIs are simple scans) and tag inheritance UI. Save-bibite file writing is landed in `bibites_io` (`3f7a593`).
3. Brain-editor mutation APIs (add/remove node/synapse, archetypes, split, step-once) — brain-dev. Read-only `Brain::differences`/`BrainDiff`/`graph_view` and species list/lineage-tree/selected-lineage APIs are now present.
4. Batch selection, challenge session/scoring — view/world/session.
5. Aggregate Biology per-organ/per-rate snapshot for the panel; present fields are public but scattered.
