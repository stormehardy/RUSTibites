//! The inspector's Stats panel in the state of its target.
//!
//! The captured session shows one state of `StatsPanel`: a living bibite without parents, traits or egg section. The game changes the
//! panel with the target (`BibiteStatsPanel.FillPanel` / `CheckAndSetAliveOrDead`): the *parents line* (a first parent, then a second
//! one), the *traits row* (radioactive, cute, source, reaper, or dead), the *egg section* instead of the bibite's rows for an egg, and
//! the *button column* (a corpse gets Revive and loses Kill, Lay and Edit; an egg and a challenge lose Lay and Revive). Each of those
//! shows or hides nodes of a `VerticalLayoutGroup` / `HorizontalLayoutGroup` hierarchy, so the rest of the panel moves.
//!
//! The layout solver ([`crate::unity_layout`], data `stats_layout_data.rs` from the prefab) lays the panel out for the captured state
//! and for the state of the target; the difference between the two moves the captured rectangles (so the captured state stays exactly
//! as captured), and a node the capture had hidden takes the rectangle of the state. The width follows the texts too: a long species
//! name widens the panel as in the game.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::app::App;
use crate::text_sdf::SdfFonts;
use crate::ui_runtime::{Node, RectTransform};
use crate::unity_layout::{self, LNode, Measure, Request, Solved};

/// The panel's root in the solver's paths.
const ROOT: &str = "StatsPanel";
const SPECIES_TEXT: &str = "StatsPanel/MainSection/SpeciesHolder/SpeciesText";
const TAG_TEXT: &str = "StatsPanel/MainSection/TagHolder/Label Holder/TagLabel";

/// What the target changes in the panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatsState {
    /// The target is an egg (`eggStatsSection` instead of `bibiteStatsSection`).
    pub egg: bool,
    /// The target is a corpse (`BibiteBody.dead`).
    pub dead: bool,
    /// `genes.parent1 != null`: the parents line shows.
    pub parent1: bool,
    /// `genes.parent2 != null`: with the second button.
    pub parent2: bool,
    /// `genes.isRad` (the "RAD" trait).
    pub rad: bool,
    /// `genes.isRain` ("CUTE").
    pub rain: bool,
    /// `genes.isSource` ("SOURCE").
    pub source: bool,
    /// `genes.isKiller` ("REAPER").
    pub killer: bool,
    /// `ChallengeManager.isChallenge`: no Lay, no Revive.
    pub challenge: bool,
}

/// The texts whose width the layout follows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatsTexts {
    /// The species name (`speciesText`).
    pub species: String,
    /// The tag line (`tagText`, or the draft while it is edited).
    pub tag: String,
}

impl StatsState {
    /// The traits row (`traitsSection`): a trait or a corpse. Eggs have none (it is part of the bibite's rows).
    #[must_use]
    pub fn traits_row(&self) -> bool {
        !self.egg && (self.rad || self.rain || self.source || self.killer || self.dead)
    }

    /// The state of the target of `app`, and the texts of the panel for it.
    #[must_use]
    pub fn of(app: &App) -> Option<(Self, StatsTexts)> {
        let world = &app.world;
        let species_name = |species: Option<bibites_core::ids::SpeciesId>| {
            species
                .and_then(|id| world.species.get(id))
                .map_or_else(|| "None".to_owned(), |record| record.name())
        };
        let tag_text = |tag: &Option<String>| match (&app.tag_edit, tag) {
            (Some(draft), _) => crate::text_field::with_caret(draft),
            (None, Some(tag)) if !tag.is_empty() => tag.clone(),
            (None, _) => "None".to_owned(),
        };
        if let Some(id) = app.selected
            && let Some(record) = world.bibites.get(id)
        {
            // A parent that is gone (rotted to meat) is a destroyed object: `!= null` is false.
            let alive = |parent: Option<bibites_core::ids::BibiteId>| {
                parent.is_some_and(|parent| world.bibites.get(parent).is_some())
            };
            let state = Self {
                egg: false,
                dead: record.body.dead,
                parent1: alive(record.meta.parent1),
                parent2: alive(record.meta.parent2),
                rad: record.meta.is_rad,
                rain: record.meta.is_rain,
                source: record.meta.is_source,
                killer: record.meta.is_killer,
                challenge: app.challenge_is_running(),
            };
            let texts = StatsTexts {
                species: species_name(record.meta.species),
                tag: tag_text(&record.meta.species_tag),
            };
            return Some((state, texts));
        }
        let id = app.selected_egg?;
        let egg = world.eggs.get(id)?;
        let alive = |parent: Option<bibites_core::ids::BibiteId>| {
            parent.is_some_and(|parent| world.bibites.get(parent).is_some())
        };
        let state = Self {
            egg: true,
            parent1: alive(egg.meta.parent1),
            parent2: alive(egg.meta.parent2),
            challenge: app.challenge_is_running(),
            ..Self::default()
        };
        let texts = StatsTexts {
            species: species_name(egg.meta.species),
            tag: tag_text(&egg.meta.species_tag),
        };
        Some((state, texts))
    }

    /// Whether `rel` (a node's path from the panel's root, `StatsPanel/...`) shows in this state, when the state decides it.
    fn forced(&self, rel: &str) -> Option<bool> {
        let rest = rel.strip_prefix("StatsPanel/")?;
        match rest {
            "MainSection/Parents Line" => Some(self.parent1),
            "MainSection/Parents Line/parent2Button" => Some(self.parent2),
            "MainSection/BibiteStats" => Some(!self.egg),
            "MainSection/EggStats" => Some(self.egg),
            "MainSection/BibiteStats/trait:" => Some(self.traits_row()),
            "MainSection/BibiteStats/trait:/Is RAD" => Some(self.rad),
            "MainSection/BibiteStats/trait:/Is DEAD" => Some(self.dead),
            "MainSection/BibiteStats/trait:/IsRAIN" => Some(self.rain),
            "MainSection/BibiteStats/trait:/IsSOURCE" => Some(self.source),
            "MainSection/BibiteStats/trait:/IsKILLER" => Some(self.killer),
            // `hideOnDead`: Kill, Lay and Edit; `showOnDead`: Revive (not in a challenge). A challenge hides Kill and Lay as well
            // (`ChallengeManager.objectsToHide`).
            "ButtonHolder/KillBibiteButton" => Some(!self.dead && !self.challenge),
            "ButtonHolder/Edit" => Some(!self.dead),
            "ButtonHolder/LayBibiteEggButton" => Some(!self.dead && !self.egg && !self.challenge),
            "ButtonHolder/ReviveCorpseButton" => Some(self.dead && !self.challenge),
            _ => None,
        }
    }
}

/// Text measuring for the solver.
struct StatsMeasure<'a>(Option<&'a SdfFonts>);

impl Measure for StatsMeasure<'_> {
    fn text_width(&self, font: &str, text: &str, size: f32, bold: bool) -> f32 {
        // TMP leaves the trailing spaces out of a text's preferred width.
        let text = text.trim_end_matches([' ', '\n']);
        let width = match self.0 {
            Some(fonts) => fonts.measure_named(Some(font), crate::ui::FontRole::Body, text, size),
            None => text.chars().count() as f32 * size * 0.52,
        };
        if bold {
            // TMP's synthetic bold advances every character but the last `boldSpacing` (0.07) of an em more, on the Roboto and the
            // Liberation faces alike (captured: "Tag:" at 22 is 43.31 regular and 47.94 bold, "Species ID:" at 14 is 69.8 and 79.61,
            // "Species:" at 26 in Liberation is 99.77 and 112.51, "Total Damages Suffered:" at 20 is 221.14 kerned and 251.95 bold).
            width + text.chars().count().saturating_sub(1) as f32 * crate::text_sdf::BOLD_SPACING * size
        } else {
            width
        }
    }

    fn line_height(&self, font: &str, size: f32) -> f32 {
        // `ascentLine - descentLine`, without the line gap.
        match self.0.and_then(|fonts| fonts.font_named(font)) {
            Some(face) => (face.ascent - face.descent) * size / face.point_size.max(1.0),
            None => size * 1.1175,
        }
    }
}

/// The measure the layout uses, for the tests that compare it with the capture.
#[cfg(test)]
pub fn measure_for_tests(fonts: &SdfFonts) -> impl Measure + '_ {
    StatsMeasure(Some(fonts))
}

/// The measure the inspector panels' layouts use (TMP's advances from the game's fonts, an estimate without them).
pub fn text_measure(fonts: Option<&SdfFonts>) -> impl Measure + '_ {
    StatsMeasure(fonts)
}

/// The game's SDF fonts for measuring when the caller has none (the window passes its own): loaded once.
pub fn fallback_fonts(root: &std::path::Path) -> Option<&'static SdfFonts> {
    static FONTS: OnceLock<Option<SdfFonts>> = OnceLock::new();
    FONTS.get_or_init(|| SdfFonts::discover(root)).as_ref()
}

/// The paths of the prefab's nodes (for telling a node the state hides from one the layout does not know).
fn known_paths() -> &'static HashSet<String> {
    static KNOWN: OnceLock<HashSet<String>> = OnceLock::new();
    KNOWN.get_or_init(|| {
        fn walk(node: &LNode, parent: &str, into: &mut HashSet<String>) {
            let path = if parent.is_empty() {
                node.name.to_owned()
            } else {
                format!("{parent}/{}", node.name)
            };
            for child in node.children {
                walk(child, &path, into);
            }
            into.insert(path);
        }
        let mut into = HashSet::new();
        walk(&crate::stats_layout_data::STATS_PANEL, "", &mut into);
        into
    })
}

/// The layout of the panel in the state of the target next to the captured one.
pub struct StatsLayout {
    /// The state's layout.
    state: Solved,
    /// The captured state's layout.
    base: Solved,
    /// The capture is a native session: its active nodes carry screen bounding boxes, which the layout's changes shift.
    native: bool,
}

impl StatsLayout {
    /// The path of a captured node from the panel's root (`StatsPanel/...`), `None` outside the panel.
    #[must_use]
    pub fn rel_of(path: &str) -> Option<&str> {
        let rel = path.split("/BibitePanels/").nth(1)?;
        (rel == ROOT || rel.starts_with("StatsPanel/")).then_some(rel)
    }

    /// Lays the panel out for `state` and `texts`, given the captured panel `capture` (the node `StatsPanel`).
    #[must_use]
    pub fn new(
        capture: &Node,
        state: &StatsState,
        texts: &StatsTexts,
        fonts: Option<&SdfFonts>,
    ) -> Self {
        let mut captured: HashMap<String, &Node> = HashMap::new();
        fn collect<'a>(node: &'a Node, into: &mut HashMap<String, &'a Node>) {
            if let Some(rel) = StatsLayout::rel_of(&node.path) {
                into.insert(rel.to_owned(), node);
            }
            for child in &node.children {
                collect(child, into);
            }
        }
        collect(capture, &mut captured);
        let measure = StatsMeasure(fonts);
        let captured_text = |rel: &str| {
            captured
                .get(rel)
                .and_then(|node| node.texts.first())
                .map(|text| text.text.clone())
        };
        let base = unity_layout::solve(
            &crate::stats_layout_data::STATS_PANEL,
            &Request {
                active: &|rel: &str, _: &LNode| captured.get(rel).is_some_and(|node| node.active),
                text: &|rel: &str, _: &LNode| captured_text(rel),
                elem: None,
            },
            &measure,
        );
        let solved = unity_layout::solve(
            &crate::stats_layout_data::STATS_PANEL,
            &Request {
                active: &|rel: &str, _: &LNode| {
                    state
                        .forced(rel)
                        .unwrap_or_else(|| captured.get(rel).is_some_and(|node| node.active))
                },
                text: &|rel: &str, _: &LNode| match rel {
                    SPECIES_TEXT => Some(texts.species.clone()),
                    TAG_TEXT => Some(texts.tag.clone()),
                    _ => captured_text(rel),
                },
                elem: None,
            },
            &measure,
        );
        Self {
            state: solved,
            base,
            native: capture.native_rect.is_some(),
        }
    }

    /// Whether a node of the panel shows in the state: `Some` for every node of the prefab, `None` for anything else.
    #[must_use]
    pub fn shows(&self, rel: &str) -> Option<bool> {
        if self.state.rects.contains_key(rel) {
            Some(true)
        } else if known_paths().contains(rel) {
            Some(false)
        } else {
            None
        }
    }

    /// What the state changes in the screen rectangle of a node a native capture has a bounding box for: how far its corner moves
    /// and how much its size grows (`dx`, `dy`, `dw`, `dh`, in canvas units, y down).
    #[must_use]
    pub fn shift(&self, rel: &str) -> Option<[f32; 4]> {
        if !self.native {
            return None;
        }
        let state = self.state.rects.get(rel)?;
        let base = self.base.rects.get(rel)?;
        Some([
            state[0] - base[0],
            state[1] - base[1],
            state[2] - base[2],
            state[3] - base[3],
        ])
    }

    /// The local rectangle (`anchoredPosition`, `sizeDelta`) of a node in the state: the captured one moved by what the state changes
    /// in the layout, or the state's own for a node the capture had hidden. `None` where nothing changes (nodes no group places).
    #[must_use]
    pub fn rect_of(&self, rel: &str, captured: &RectTransform) -> Option<RectTransform> {
        let state = *self.state.rects.get(rel)?;
        // A native capture positions its active nodes by their bounding boxes (see [`Self::shift`]).
        if self.native && self.base.rects.contains_key(rel) {
            return None;
        }
        if rel == ROOT {
            // The panel fits its content: only its size changes.
            let base = *self.base.rects.get(rel)?;
            let mut rect = *captured;
            rect.size_delta.x += state[2] - base[2];
            rect.size_delta.y += state[3] - base[3];
            return Some(rect);
        }
        if !self.state.placed.contains(rel) {
            return None;
        }
        let local = |solved: &Solved| -> Option<(f32, f32, f32, f32)> {
            let rect = *solved.rects.get(rel)?;
            let parent = *solved.rects.get(rel.rsplit_once('/')?.0)?;
            let (x, y) = (rect[0] - parent[0], rect[1] - parent[1]);
            let pivot = (captured.pivot.x, captured.pivot.y);
            Some((
                x + pivot.0 * rect[2],
                -(y + (1.0 - pivot.1) * rect[3]),
                rect[2],
                rect[3],
            ))
        };
        let (ax, ay, w, h) = local(&self.state)?;
        let mut rect = *captured;
        match local(&self.base) {
            Some((bx, by, bw, bh)) => {
                rect.anchored_position.x += ax - bx;
                rect.anchored_position.y += ay - by;
                rect.size_delta.x += w - bw;
                rect.size_delta.y += h - bh;
            }
            None => {
                // Hidden in the capture: its stored rectangle is the prefab's (laid out for another state).
                rect.anchored_position.x = ax;
                rect.anchored_position.y = ay;
                rect.size_delta.x = w;
                rect.size_delta.y = h;
                rect.anchor_min.x = 0.0;
                rect.anchor_min.y = 1.0;
                rect.anchor_max.x = 0.0;
                rect.anchor_max.y = 1.0;
            }
        }
        Some(rect)
    }
}
