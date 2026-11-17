//! The layout of a captured inspector panel (`BibitePanels/<name>`) in the state it is in.
//!
//! A native capture is one state of a panel: the species name it was taken with, the holder size of the brain it showed. The game lays
//! the panel out again whenever a text or a size changes (`HorizontalLayoutGroup` / `VerticalLayoutGroup` / `ContentSizeFitter`), so a
//! longer species name widens the row it is in and a taller diagram pushes everything under it down. This solves the panel's prefab
//! (`tools/unity/gen_layout_data.py`, [`crate::unity_layout`]) for the captured state and for the current one; the difference moves the
//! captured screen rectangles (see [`PanelLayout::shift`]). The Stats panel does the same with its states (`stats_states`).

use std::collections::HashMap;

use crate::text_sdf::SdfFonts;
use crate::ui_runtime::Node;
use crate::unity_layout::{self, ElemOverride, LNode, Request, Solved};

/// A panel laid out for a state next to the captured one.
pub struct PanelLayout {
    /// The current state's layout.
    state: Solved,
    /// The captured state's layout.
    base: Solved,
    /// The capture is a native session: its active nodes carry screen bounding boxes, which the layout's changes shift.
    native: bool,
}

impl PanelLayout {
    /// The path of a captured node from its panel's root (`BrainPanel/...`), `None` outside the inspector panels.
    #[must_use]
    pub fn rel_of(path: &str) -> Option<&str> {
        path.split("/BibitePanels/").nth(1)
    }

    /// Lays `data` (the prefab of the panel whose captured node is `capture`) out as captured and as the state says: `text` gives the
    /// text a node shows now (given its path from the root and its captured node); `captured_elem` and `elem` replace a
    /// `LayoutElement` the code sets at run time, as it was when the panel was captured and as it is now (the prefab's values are the
    /// ones before the code touched them).
    #[must_use]
    pub fn new(
        capture: &Node,
        data: &'static LNode,
        text: &dyn Fn(&str, &Node) -> Option<String>,
        captured_elem: Option<&ElemOverride<'_>>,
        elem: Option<&ElemOverride<'_>>,
        fonts: Option<&SdfFonts>,
    ) -> Self {
        fn collect<'a>(node: &'a Node, into: &mut HashMap<String, &'a Node>) {
            if let Some(rel) = PanelLayout::rel_of(&node.path) {
                into.insert(rel.to_owned(), node);
            }
            for child in &node.children {
                collect(child, into);
            }
        }
        let mut captured: HashMap<String, &Node> = HashMap::new();
        collect(capture, &mut captured);
        let measure = crate::stats_states::text_measure(fonts);
        let captured_text = |rel: &str| {
            captured
                .get(rel)
                .and_then(|node| node.texts.first())
                .map(|text| text.text.clone())
        };
        let active = |rel: &str, _: &LNode| captured.get(rel).is_some_and(|node| node.active);
        let base = unity_layout::solve(
            data,
            &Request {
                active: &active,
                text: &|rel: &str, _: &LNode| captured_text(rel),
                elem: captured_elem,
            },
            &measure,
        );
        let state = unity_layout::solve(
            data,
            &Request {
                active: &active,
                text: &|rel: &str, _: &LNode| {
                    captured
                        .get(rel)
                        .and_then(|node| text(rel, node))
                        .or_else(|| captured_text(rel))
                },
                elem,
            },
            &measure,
        );
        Self {
            state,
            base,
            native: capture.native_rect.is_some(),
        }
    }

    /// What the state changes in the screen rectangle of a node a native capture has a bounding box for: how far its corner moves and
    /// how much its size grows (`dx`, `dy`, `dw`, `dh`, in canvas units, y down); `None` where nothing changes.
    #[must_use]
    pub fn shift(&self, rel: &str) -> Option<[f32; 4]> {
        if !self.native {
            return None;
        }
        let state = self.state.rects.get(rel)?;
        let base = self.base.rects.get(rel)?;
        let shift = [
            state[0] - base[0],
            state[1] - base[1],
            state[2] - base[2],
            state[3] - base[3],
        ];
        shift.iter().any(|v| v.abs() > 1e-3).then_some(shift)
    }
}
