//! The Bibite Editor's model (`BibiteEditorPanel`, `BibiteEditorPreview`, `BibiteGrowthGraph`): the 35 genes, the template's name and
//! description, its brain, the rules that decide whether it can be saved, the file it is saved to and the numbers its preview shows.
//!
//! The editor works on a copy of a template (`EditTemplate(template)`) or on a new one (`EditTemplate(null)`: the genes at their
//! defaults, named "Bibite template"). A gene slider never goes out of its range (`canGoOutOfBounds` is off for the gene settings),
//! so every gene stays between its minimum and maximum however it is set; Save writes `<name>.bb8template` in the templates folder
//! (`SaveSystem.SaveTemplate`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bibites_core::brain::Brain;
use bibites_core::genome::{GENE_DEFS, Gene, Genome, GenomeMeta, N_GENES};
use bibites_core::world::TemplateSource;

/// `speciesName.DefaultValue`: what a new template is called.
pub const DEFAULT_NAME: &str = "Bibite template";

/// `Path.GetInvalidFileNameChars()` on Windows, where the game's name check (`containsABadCharacter`) runs.
#[must_use]
pub fn is_bad_file_name_char(c: char) -> bool {
    matches!(c, '"' | '<' | '>' | '|' | ':' | '*' | '?' | '\\' | '/') || u32::from(c) < 32
}

/// What `CheckExistingBibite` decides about the name (the three warning icons and whether Save works).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NameCheck {
    /// The name can be a file name and is not empty (`nameValid`).
    pub valid: bool,
    /// The name has a character a file name cannot have (`incorrectWarning`).
    pub bad_character: bool,
    /// The name is one of the game's own templates (`existingDefaultWarning`), which cannot be overwritten.
    pub official: bool,
    /// A template of that name is saved already and saving would replace it (`existingWarning`). Like the game's, it also shows for
    /// an empty name, and keeps its last state while the name is an official one.
    pub exists: bool,
}

impl NameCheck {
    /// `CheckExistingBibite(newName)`, from the previous state (the `exists` icon is left as it was for an official name).
    #[must_use]
    pub fn of(
        name: &str,
        official: &[String],
        previous_exists: bool,
        file_exists: impl Fn(&str) -> bool,
    ) -> Self {
        let bad_character = name.chars().any(is_bad_file_name_char);
        let valid = !bad_character && !name.is_empty();
        let lower = name.to_lowercase();
        if official.contains(&lower) {
            return Self {
                valid: false,
                bad_character,
                official: true,
                exists: previous_exists,
            };
        }
        // The file check does not touch `valid`: a name that is taken can still be saved, over the old one.
        let saved_under_new_name = !name.is_empty() && !file_exists(name);
        Self {
            valid,
            bad_character,
            official: false,
            exists: !saved_under_new_name,
        }
    }
}

/// The numbers and the curve of the preview panel (`BibiteEditorPreview.UpdatePreview`, `BibiteGrowthGraph.UpdateCurve`).
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    /// The growth at birth over the growth at maturity, 0 to 1.
    pub maturity_at_birth: f32,
    /// `AreaAtMature` (u²).
    pub size_at_maturity: f32,
    /// `LengthAtMature` (u).
    pub length_at_maturity: f32,
    /// The share of the body's area each organ takes: womb, stomach, fat, armor, throat, jaw, move muscles.
    pub womb: f32,
    /// See `womb`.
    pub stomach: f32,
    /// See `womb`.
    pub fat: f32,
    /// See `womb`.
    pub armor: f32,
    /// See `womb`.
    pub throat: f32,
    /// See `womb`.
    pub jaw: f32,
    /// See `womb`.
    pub move_muscles: f32,
    /// The growth curve: 50 points, `(maturity, growth speed in u²/s)`.
    pub curve: Vec<(f32, f32)>,
}

/// The lower case names of the game's own templates in `folder` (`GameManager.defaultBibites`: the templates the game ships and
/// writes into the folder at start, whose names a saved template cannot take).
#[must_use]
pub fn official_names(folder: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for path in entries.filter_map(|entry| entry.ok().map(|entry| entry.path())) {
        let is_template = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("bb8template"));
        if !is_template {
            continue;
        }
        let Ok(loaded) = bibites_io::load_template(&path) else {
            continue;
        };
        if loaded.template.is_official {
            let name = loaded.template.name.clone().unwrap_or_else(|| {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            names.push(name.to_lowercase());
        }
    }
    names
}

/// The brain of a new template (`EditTemplate(null)`: `NEATBrain.SetBaseNeuronsTypeAndDesc` on a fresh array): the 33 inputs and the 15
/// outputs, every bias zero, no hidden nodes and no synapses.
#[must_use]
pub fn blank_brain() -> Brain {
    let mut nodes = Brain::starter().to_node_records();
    for node in &mut nodes {
        node.base_activation = 0.0;
    }
    Brain::from_records(&nodes, &[]).unwrap_or_else(|_| Brain::starter())
}

/// `BibiteGrowth.GrowthFunction`: how fast a bibite of this maturity grows.
#[must_use]
pub fn growth_function(x: f32, scale: f32, factor: f32, exponent: f32) -> f32 {
    scale / (1.0 + factor * x.powf(exponent))
}

/// The Bibite Editor's state.
#[derive(Clone, Debug)]
pub struct TemplateEditor {
    genes: [f32; N_GENES],
    /// `speciesName.val`: the template's name (the file is `<name>.bb8template`).
    name: String,
    /// `speciesDesc.val`.
    pub description: String,
    /// The template's brain, saved as it is (the brain tab is not implemented yet).
    pub brain: Brain,
    /// Where the brain editor had each node (`nodeAnchors`), carried through a save.
    pub anchors: BTreeMap<i64, bibites_io::model::AnchorJson>,
    /// Any gene or the brain was changed since the editor opened (`anyChange`).
    pub any_change: bool,
    /// What the name check says now.
    pub check: NameCheck,
    /// The lower case names of the game's own templates (`GameManager.defaultBibites`).
    official: Vec<String>,
}

impl TemplateEditor {
    /// `EditTemplate(null)`: a template at the genes' defaults, with the editor's default name and no description. `brain` is what
    /// the new template starts with ([`blank_brain`]).
    #[must_use]
    pub fn new_empty(
        brain: Brain,
        official: Vec<String>,
        file_exists: impl Fn(&str) -> bool,
    ) -> Self {
        let mut editor = Self {
            genes: Genome::defaults().genes,
            name: DEFAULT_NAME.to_owned(),
            description: String::new(),
            brain,
            anchors: BTreeMap::new(),
            any_change: false,
            check: NameCheck::default(),
            official,
        };
        editor.check_name(file_exists);
        editor
    }

    /// `EditTemplate(template)`: the template's genes (kept inside their ranges), `name`, description, brain and anchors. The editor
    /// does not keep the template's generation: `SaveTemplate` writes a new template (generation 0).
    #[must_use]
    pub fn from_template(
        template: &bibites_io::LoadedGenome,
        name: String,
        official: Vec<String>,
        file_exists: impl Fn(&str) -> bool,
    ) -> Self {
        Self::from_parts(
            template.genome.genes,
            name,
            template.template.description.clone().unwrap_or_default(),
            template.brain.clone(),
            template.template.node_anchors.clone(),
            official,
            file_exists,
        )
    }

    /// `new BibiteTemplate(path)` and `EditTemplate`: the Library's selected `.bb8template` (named by its `name`, empty when it has
    /// none) or saved `.bb8` bibite (named after its file).
    pub fn from_file(
        path: &Path,
        official: Vec<String>,
        file_exists: impl Fn(&str) -> bool,
    ) -> Result<Self, String> {
        let loaded = bibites_io::load_template(path).map_err(|error| error.to_string())?;
        let saved_bibite = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("bb8"));
        let name = if saved_bibite {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            loaded.template.name.clone().unwrap_or_default()
        };
        Ok(Self::from_template(&loaded, name, official, file_exists))
    }

    /// `EditTemplate(template)` for a template made of its parts (a bibite's or a species'): the genes are kept inside their ranges.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        mut genes: [f32; N_GENES],
        name: String,
        description: String,
        brain: Brain,
        anchors: BTreeMap<i64, bibites_io::model::AnchorJson>,
        official: Vec<String>,
        file_exists: impl Fn(&str) -> bool,
    ) -> Self {
        for (gene, def) in genes.iter_mut().zip(GENE_DEFS.iter()) {
            *gene = gene.clamp(def.min, def.max);
        }
        let mut editor = Self {
            genes,
            name,
            description,
            brain,
            anchors,
            any_change: false,
            check: NameCheck::default(),
            official,
        };
        editor.check_name(file_exists);
        editor
    }

    /// The 35 genes as the sliders show them.
    #[must_use]
    pub fn genes(&self) -> &[f32; N_GENES] {
        &self.genes
    }

    /// `FloatSettingSlider` of gene `gene` set to `value` (`SetValue`: never out of the gene's range).
    pub fn set_gene(&mut self, gene: usize, value: f32) {
        let def = &GENE_DEFS[gene];
        let value = value.clamp(def.min, def.max);
        if self.genes[gene] != value {
            self.genes[gene] = value;
            self.any_change = true;
        }
    }

    /// The row's reset button (`ResetValue`: back to `DefaultValue`).
    pub fn reset_gene(&mut self, gene: usize) {
        self.set_gene(gene, GENE_DEFS[gene].default);
    }

    /// The template's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The name field changed (`speciesName.Subscribe(CheckExistingBibite)`).
    pub fn set_name(&mut self, name: impl Into<String>, file_exists: impl Fn(&str) -> bool) {
        self.name = name.into();
        self.check_name(file_exists);
    }

    /// `CheckExistingBibite` for the current name.
    pub fn check_name(&mut self, file_exists: impl Fn(&str) -> bool) {
        self.check = NameCheck::of(&self.name, &self.official, self.check.exists, file_exists);
    }

    /// `UpdateValidity`: whether Save and Save & Submit work (`nameValid && brainEditor.valid`).
    #[must_use]
    pub fn can_save(&self) -> bool {
        self.check.valid
    }

    /// The genome the template has.
    #[must_use]
    pub fn genome(&self) -> Genome {
        Genome { genes: self.genes }
    }

    /// The path `<folder>/<name>.bb8template` (`GetSavePath`).
    #[must_use]
    pub fn save_path(&self, folder: &Path) -> PathBuf {
        folder.join(format!("{}.bb8template", self.name))
    }

    /// The template as a spawnable source.
    #[must_use]
    pub fn source(&self) -> TemplateSource {
        TemplateSource::new(
            format!("{}.bb8template", self.name),
            self.genome(),
            GenomeMeta::default(),
            self.brain.clone(),
        )
    }

    /// `SaveTemplate`: writes the template into `folder`, indented when `pretty` (`UserSettings.FormatBibiteJSON`). Returns the file
    /// written and the template (what `Save & Submit` hands to its caller).
    pub fn save(
        &mut self,
        folder: &Path,
        pretty: bool,
    ) -> Result<(PathBuf, TemplateSource), String> {
        if !self.can_save() {
            return Err("the name cannot be saved under".to_owned());
        }
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
        let path = self.save_path(folder);
        // `new BibiteTemplate { name, speciesName = the name as typed, description, genes }`: the generation is 0.
        let meta = bibites_io::TemplateMeta {
            name: Some(self.name.clone()),
            species_name: Some(self.name.clone()),
            description: Some(self.description.clone()),
            is_official: false,
            node_anchors: self.anchors.clone(),
        };
        bibites_io::save_template_pretty(
            &path,
            &self.genome(),
            &GenomeMeta::default(),
            &self.brain,
            &meta,
            pretty,
        )
        .map_err(|error| error.to_string())?;
        let exists = |name: &str| folder.join(format!("{name}.bb8template")).is_file();
        self.check_name(exists);
        Ok((path, self.source()))
    }

    /// The numbers and the growth curve of the preview panel for the genes as they are.
    #[must_use]
    pub fn preview(&self) -> Preview {
        const POINTS: usize = 50;
        let genome = self.genome();
        let genes = &self.genes;
        let at_birth = genome.growth_at_birth();
        let at_mature = genome.growth_at_mature();
        // `BibiteGrowthGraph.UpdateCurve`: the curve's scale, factor and exponent from the genes, its x range from the
        // maturity at birth to where the curve has fallen to a tenth of the way, in steps the graph's 11 lines fit.
        let size_ratio = genes[Gene::SizeRatio as usize];
        let scale = genes[Gene::SpeedRatio as usize]
            * size_ratio
            * size_ratio
            * genes[Gene::GrowthScale as usize]
            / 100.0
            * 80.0;
        let factor = genes[Gene::GrowthMaturityFactor as usize];
        let exponent = genes[Gene::GrowthMaturityExponent as usize];
        let n_x = 11_usize;
        let x_min = at_birth / at_mature;
        let at_start = scale.max(1e-5) / (1.0 + factor * x_min.powf(exponent));
        let x_tenth = ((10.0 * scale / at_start - 1.0).max(0.0) / factor).powf(1.0 / exponent);
        let mut steps = 2_i32;
        for candidate in [2_i32, 3, 4, 5, 6] {
            if (candidate as f32) < n_x as f32 / x_tenth {
                steps = candidate;
                continue;
            }
            break;
        }
        let x_max = (n_x - 1) as f32 / steps as f32;
        let curve = (0..POINTS)
            .map(|i| {
                let x = x_min + (x_max - x_min) * i as f32 / (POINTS - 1) as f32;
                (x, growth_function(x, scale, factor, exponent))
            })
            .collect();
        Preview {
            maturity_at_birth: at_birth / at_mature,
            size_at_maturity: genome.area_at_mature(),
            length_at_maturity: genome.length_at_mature(),
            womb: genome.organ_area_portion(Gene::WombWAG),
            stomach: genome.organ_area_portion(Gene::StomachWAG),
            fat: genome.organ_area_portion(Gene::FatWAG),
            armor: genome.organ_area_portion(Gene::ArmorWAG),
            throat: genome.organ_area_portion(Gene::ThroatWAG),
            jaw: genome.organ_area_portion(Gene::MouthMusclesWAG),
            move_muscles: genome.organ_area_portion(Gene::MoveMusclesWAG),
            curve,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> bool {
        false
    }

    fn fresh() -> TemplateEditor {
        TemplateEditor::new_empty(Brain::starter(), vec!["basic bibite".to_owned()], none)
    }

    #[test]
    fn a_new_template_has_the_defaults_and_the_editors_default_name() {
        let editor = fresh();
        assert_eq!(editor.name(), "Bibite template");
        assert_eq!(editor.description, "");
        for (value, def) in editor.genes().iter().zip(GENE_DEFS.iter()) {
            assert_eq!(*value, def.default, "{}", def.name);
        }
        assert!(!editor.any_change);
        assert!(editor.can_save());
        assert_eq!(
            editor.check,
            NameCheck {
                valid: true,
                bad_character: false,
                official: false,
                exists: false
            }
        );
    }

    #[test]
    fn a_gene_never_leaves_its_range_and_a_change_is_noted() {
        let mut editor = fresh();
        let diet = Gene::Diet as usize;
        editor.set_gene(diet, 0.4);
        assert_eq!(editor.genes()[diet], 0.4);
        assert!(editor.any_change);
        // Past either end the slider stops at the end.
        editor.set_gene(diet, 5.0);
        assert_eq!(editor.genes()[diet], GENE_DEFS[diet].max);
        editor.set_gene(diet, -5.0);
        assert_eq!(editor.genes()[diet], GENE_DEFS[diet].min);
        // The reset button goes back to the default.
        editor.reset_gene(diet);
        assert_eq!(editor.genes()[diet], GENE_DEFS[diet].default);
        // Setting what it already is changes nothing.
        let mut still = fresh();
        still.set_gene(diet, GENE_DEFS[diet].default);
        assert!(!still.any_change);
    }

    #[test]
    fn a_template_opens_with_its_genes_inside_their_ranges() {
        let mut loaded = bibites_io::LoadedGenome::starter_template("Mine");
        loaded.genome.genes[Gene::Diet as usize] = 7.0;
        loaded.genome.genes[Gene::LayTime as usize] = 0.0;
        loaded.template.description = Some("a note".to_owned());
        let editor = TemplateEditor::from_template(&loaded, "Mine".to_owned(), Vec::new(), none);
        assert_eq!(editor.name(), "Mine");
        assert_eq!(editor.description, "a note");
        assert_eq!(
            editor.genes()[Gene::Diet as usize],
            GENE_DEFS[Gene::Diet as usize].max
        );
        assert_eq!(
            editor.genes()[Gene::LayTime as usize],
            GENE_DEFS[Gene::LayTime as usize].min
        );
        assert!(!editor.any_change, "opening a template is not a change");
    }

    #[test]
    fn a_new_templates_brain_is_the_bare_base_neurons() {
        let brain = blank_brain();
        // `NInputs + NOutputs` nodes, no hidden ones, no synapses, every bias zero.
        assert_eq!(brain.node_count(), 33 + 15);
        assert_eq!(brain.hidden_count(), 0);
        assert_eq!(brain.synapse_count(), 0);
        assert!(
            brain
                .to_node_records()
                .iter()
                .all(|node| node.base_activation == 0.0)
        );
    }

    #[test]
    fn the_library_edits_a_template_by_its_name_and_a_saved_bibite_by_its_file() {
        let dir =
            std::env::temp_dir().join(format!("rustibites_editor_file_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut loaded = bibites_io::LoadedGenome::starter_template("Library one");
        loaded.template.description = Some("from the library".to_owned());
        loaded.meta.generation = 12;
        let template = dir.join("file name.bb8template");
        bibites_io::save_template(
            &template,
            &loaded.genome,
            &loaded.meta,
            &loaded.brain,
            &loaded.template,
        )
        .expect("writes");
        // A template is named by its `name`, not by its file; its generation does not come along.
        let editor = TemplateEditor::from_file(&template, Vec::new(), none).expect("reads");
        assert_eq!(editor.name(), "Library one");
        assert_eq!(editor.description, "from the library");
        assert_eq!(editor.brain.node_count(), loaded.brain.node_count());
        // A saved bibite (`.bb8`) is named after its file.
        let saved = dir.join("my saved.bb8");
        bibites_io::save_genome(&saved, &loaded.genome, &loaded.meta, &loaded.brain)
            .expect("writes");
        let editor = TemplateEditor::from_file(&saved, Vec::new(), none).expect("reads");
        assert_eq!(editor.name(), "my saved");
        assert_eq!(editor.brain.synapse_count(), loaded.brain.synapse_count());
        // A file that is not a template is an error, not an empty editor.
        let junk = dir.join("junk.bb8template");
        std::fs::write(&junk, "not json").expect("writes");
        assert!(TemplateEditor::from_file(&junk, Vec::new(), none).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_name_check_follows_the_game() {
        let official = vec!["basic bibite".to_owned()];
        let check = |name: &str, previous: bool, taken: &[&str]| {
            NameCheck::of(name, &official, previous, |candidate| {
                taken.contains(&candidate)
            })
        };
        // A plain name: fine, and nothing in the way.
        assert_eq!(
            check("Fast one", false, &[]),
            NameCheck {
                valid: true,
                bad_character: false,
                official: false,
                exists: false
            }
        );
        // A name of a template that is saved already can be saved over it, with the warning.
        assert_eq!(
            check("Fast one", false, &["Fast one"]),
            NameCheck {
                valid: true,
                bad_character: false,
                official: false,
                exists: true
            }
        );
        // A character a file name cannot have.
        for bad in [
            "a/b",
            "a:b",
            "a*b",
            "a?b",
            "a\"b",
            "a<b",
            "a>b",
            "a|b",
            "a\\b",
            "tab\there",
        ] {
            let result = check(bad, false, &[]);
            assert!(result.bad_character && !result.valid, "{bad:?}");
        }
        // Empty: not valid, and (as in the game) the existing warning shows, as no file can be saved under it.
        let empty = check("", false, &[]);
        assert!(!empty.valid && !empty.bad_character && empty.exists);
        // The game's own templates cannot be overwritten, whatever the case; the existing icon stays as it was.
        let official_name = check("BASIC Bibite", true, &[]);
        assert_eq!(
            official_name,
            NameCheck {
                valid: false,
                bad_character: false,
                official: true,
                exists: true
            }
        );
        assert!(!check("BASIC Bibite", false, &[]).exists);
    }

    #[test]
    fn saving_writes_the_template_and_reads_back_the_same() {
        let dir = std::env::temp_dir().join(format!("rustibites_editor_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut editor = fresh();
        editor.set_name("Editor test", |name| {
            dir.join(format!("{name}.bb8template")).is_file()
        });
        editor.description = "made in the editor".to_owned();
        editor.set_gene(Gene::Diet as usize, 0.8);
        editor.set_gene(Gene::LayTime as usize, 31.0);
        let (path, source) = editor.save(&dir, false).expect("saves");
        assert_eq!(path, dir.join("Editor test.bb8template"));
        assert_eq!(source.name, "Editor test.bb8template");
        // The file is a template the loader reads back with the edited genes, the name and the description.
        let loaded = bibites_io::load_template(&path).expect("reads back");
        assert_eq!(loaded.genome.genes[Gene::Diet as usize], 0.8);
        assert_eq!(loaded.genome.genes[Gene::LayTime as usize], 31.0);
        assert_eq!(loaded.template.name.as_deref(), Some("Editor test"));
        // `new BibiteTemplate { name, speciesName, description, genes }`: the species name is the name as typed (no species
        // casing) and the generation is the new template's 0.
        assert_eq!(loaded.template.species_name.as_deref(), Some("Editor test"));
        assert_eq!(loaded.meta.generation, 0);
        assert_eq!(
            loaded.template.description.as_deref(),
            Some("made in the editor")
        );
        assert_eq!(loaded.brain.node_count(), editor.brain.node_count());
        assert_eq!(loaded.brain.synapse_count(), editor.brain.synapse_count());
        // Saved, the name is taken now: saving again overwrites with the warning up.
        assert!(editor.check.exists);
        editor.save(&dir, true).expect("saves over it");
        let pretty = std::fs::read_to_string(&path).expect("file");
        assert!(
            pretty.contains('\n'),
            "indented when the player's setting says so"
        );
        // A name that cannot be saved under is refused and writes nothing.
        editor.set_name("bad/name", none);
        assert!(editor.save(&dir, false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_preview_numbers_follow_the_genes() {
        let mut editor = fresh();
        let base = editor.preview();
        assert_eq!(base.curve.len(), 50);
        // The curve runs from the maturity at birth, falling as the bibite matures.
        assert!((base.curve[0].0 - base.maturity_at_birth).abs() < 1e-6);
        assert!(base.curve[0].1 > base.curve[49].1);
        assert!(base.maturity_at_birth > 0.0 && base.maturity_at_birth < 1.0);
        // A bigger bibite has a bigger area and length at maturity.
        editor.set_gene(Gene::SizeRatio as usize, 2.0);
        let big = editor.preview();
        assert!(big.size_at_maturity > base.size_at_maturity);
        assert!(big.length_at_maturity > base.length_at_maturity);
        // More stomach gene, more of the body is stomach.
        let before = editor.preview().stomach;
        editor.set_gene(
            Gene::StomachWAG as usize,
            GENE_DEFS[Gene::StomachWAG as usize].max,
        );
        assert!(editor.preview().stomach > before);
        // The shares are fractions of the body.
        let shares = editor.preview();
        let total = shares.womb
            + shares.stomach
            + shares.fat
            + shares.armor
            + shares.throat
            + shares.jaw
            + shares.move_muscles;
        assert!(total > 0.0 && total <= 1.0001, "{total}");
    }
}
