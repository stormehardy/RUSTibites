#!/usr/bin/env python3
"""Build ``tools/assets/ui_expectations.json`` for parity-dev's asset extraction.

The expectation list answers: *which UI sprites, fonts, icons and materials must
the extracted ``assets/game/`` batch contain so the viewer can reproduce The
Bibites' UI 1:1?*

Two inputs are merged:

1. **A curated logical catalogue** derived from ``docs/design/UI_PARITY.md``
   (the UI inventory: panels, widgets, HUD, overlays, tooltips, editors).  These
   are the assets the viewer *needs*, named with the scheme in
   ``docs/ASSETS_UI.md``.
2. **A scan of the decompiled C#** (``Here/UIScripts*/**`` plus the management /
   rendering / Steam UI scripts).  The scan records every sprite/font field and
   every code assignment, so each expectation carries ``source`` references and
   the JSON also contains a ``field_inventory`` of all UI graphic fields — the
   ones that are only wired through Unity prefabs are flagged ``prefab_only``
   for parity-dev to resolve while extracting the level scenes.

Run from the repo root (or anywhere; paths are derived from this file)::

    python tools/assets/build_ui_expectations.py
    python tools/assets/build_ui_expectations.py --check   # print only

Docs-only/tools-only task: this script writes only ``tools/assets/ui_expectations.json``.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = ROOT / "Here"
OUT = ROOT / "tools" / "assets" / "ui_expectations.json"

# ---------------------------------------------------------------------------
# Naming scheme (documented in docs/ASSETS_UI.md)
# ---------------------------------------------------------------------------
NAMING = {
    "root": "assets/game/",
    "ui": "ui/<usage>/<logical_name>[.<ext>]",
    "usage": [
        "panel", "button", "toggle", "slider", "progress", "dropdown", "input",
        "tabs", "scroll", "graph", "chart", "icon", "cursor", "glow", "overlay",
        "hud", "tooltip", "popup", "notification", "menu", "font", "material",
        "preview",
    ],
    "conventions": [
        "lowercase snake_case logical names; no spaces",
        "sprites are PNG, fonts TTF/OTF (or the TMP atlas if conversion is lossy)",
        "9-slice borders and pivot/PPU belong in assets/game/manifest.json, not in the name",
        "groups nest one level: ui/icon/brain_input/00 (two-digit index)",
        "in-world overlays use the same ui/ tree (they are UI-ish art)",
    ],
}

# ---------------------------------------------------------------------------
# Curated catalogue helpers
# ---------------------------------------------------------------------------
ENTRIES: list[dict] = []


def E(
    logical: str,
    usage: str,
    priority: str,
    *,
    surfaces: tuple[str, ...] = (),
    variants: tuple[str, ...] = (),
    nine_slice: bool = False,
    pivot: tuple[float, float] | None = None,
    extraction: str | None = None,
    sources: tuple[dict, ...] = (),
    prefab_only: bool = True,
    optional: bool = False,
    notes: str = "",
) -> None:
    entry_id = logical.replace("/", ".").replace("[", "_").replace("]", "")
    if extraction is None:
        extraction = "shader_material" if usage == "material" else "sprite_png"
    ENTRIES.append(
        {
            "id": entry_id,
            "logical": logical,
            "usage": usage,
            "priority": priority,
            "surfaces": list(surfaces),
            "variants": list(variants),
            "nine_slice": nine_slice,
            "pivot": list(pivot) if pivot else None,
            "extraction": extraction,
            "prefab_only": prefab_only,
            "optional": optional,
            "source": [dict(s) for s in sources],
            "notes": notes,
        }
    )


def src(file: str, line: int | None = None, field: str | None = None, ref: str = "prefab") -> dict:
    out = {"file": file, "ref": ref}
    if line is not None:
        out["line"] = line
    if field is not None:
        out["field"] = field
    return out


# -- shared panel chrome ----------------------------------------------------
PANELS = [
    # (slug, priority, surfaces, notes)
    ("hud_time", "P0", ("W1", "W2"), "time/TPS readout + speed slider cluster"),
    ("hud_ecology", "P0", ("W5",), "rightmost quick-statistics panel"),
    ("hud_statusbars", "P0", ("W7",), "floating health/energy bars"),
    ("hud_selection", "P0", ("W3", "W8"), "selection rectangle + context buttons"),
    ("bibite_stats", "P0", ("B1",), "Bibite Stats panel"),
    ("bibite_genes", "P0", ("B2",), "Bibite Genes panel"),
    ("bibite_biology", "P0b", ("B3",), "Biology organ panel (largest in-world panel)"),
    ("bibite_brain", "P0b", ("B4",), "simple brain node graph panel"),
    ("bibite_brain_expanded", "P3", ("B5",), "expanded force-directed brain view"),
    ("selection_multi", "P1", ("W9",), "rubber-band multi-selection panel"),
    ("escape_menu", "P1", ("W10",), "in-game escape menu"),
    ("stats_full", "P1", ("A1",), "full statistics/wedge-chart panel"),
    ("graphs", "P1", ("A3",), "historical/gene graphs panel"),
    ("lineage", "P2", ("A4", "A5"), "ancestral lineage + lineage tree window"),
    ("species_info", "P1", ("B7",), "species info/genes/brain tabs"),
    ("dynamic_settings", "P2", ("E1",), "in-world dynamic settings editor"),
    ("zones_editor", "P2", ("E2",), "zone list/editor with previews"),
    ("scenario_editor", "P2", ("M13",), "full-screen scenario settings editor"),
    ("template_browser", "P2", ("M14",), "bibite template picker"),
    ("genome_editor", "P3", ("M15",), "bibite genome editor"),
    ("brain_editor", "P3", ("M16",), "brain graph editor"),
    ("load_game", "P2", ("M6",), "load-game panel"),
    ("save_game", "P2", ("M7",), "save-game panel"),
    ("main_menu", "P2", ("M2",), "main menu shell"),
    ("title_screen", "P3", ("M1",), "first-launch title splash"),
    ("loading", "P3", ("M11",), "loading progress screen"),
    ("tooltip", "P0", ("D1",), "cursor-following tooltip"),
    ("popup", "P1", ("D4", "D5", "D6"), "error/warning/choice modal"),
    ("notification", "P1", ("W11",), "toast notification"),
    ("tool_panel", "P1", ("E7", "E8", "E9"), "rad/phero/colour-killer tool panel"),
]
for slug, prio, surfaces, note in PANELS:
    E(
        f"ui/panel/{slug}_frame",
        "panel",
        prio,
        surfaces=surfaces,
        variants=("9-slice",),
        nine_slice=True,
        pivot=(0.5, 0.5),
        notes=f"Panel background/frame for {slug}: {note}.",
    )
    if slug in {
        "bibite_stats", "bibite_genes", "bibite_biology", "bibite_brain",
        "selection_multi", "escape_menu", "stats_full", "graphs", "lineage",
        "species_info", "dynamic_settings", "zones_editor", "scenario_editor",
        "template_browser", "genome_editor", "brain_editor", "load_game",
        "save_game", "tool_panel",
    }:
        E(
            f"ui/panel/{slug}_header",
            "panel",
            prio,
            surfaces=surfaces,
            variants=("9-slice",),
            nine_slice=True,
            notes=f"Header/title bar of the {slug} panel (shared style expected; included for screenshot parity).",
        )

# -- shared widgets ---------------------------------------------------------
for name, variants, usage, nine, prio, note in [
    ("button", ("normal", "hover", "pressed", "disabled"), "button", True, "P0", "Standard rounded button, 9-slice."),
    ("button_small", ("normal", "hover", "pressed", "disabled"), "button", True, "P0", "Compact button used in list rows and toolbars."),
    ("button_icon", ("normal", "hover", "pressed"), "button", False, "P0", "Square icon-only button (play/pause, close, tabs)."),
    ("close_x", ("normal", "hover"), "button", False, "P0", "Close X used on panels/popups."),
    ("toggle", ("on", "off"), "toggle", True, "P0", "Boolean toggle (checkbox style)."),
    ("toggle_icon", ("on", "off"), "toggle", False, "P1", "Two-icon toggle used by sub-settings masters."),
    ("slider_track", ("normal",), "slider", True, "P0", "Slider track/background."),
    ("slider_fill", ("normal",), "slider", True, "P0", "Slider filled portion (Image.filled)."),
    ("slider_knob", ("normal",), "slider", False, "P0", "Slider handle/knob."),
    ("dropdown_bg", ("normal",), "dropdown", True, "P0", "Dropdown closed background."),
    ("dropdown_arrow", ("normal",), "dropdown", False, "P0", "Dropdown expand arrow."),
    ("dropdown_item", ("normal", "hover", "selected"), "dropdown", True, "P0", "Dropdown option row."),
    ("input_bg", ("normal", "focused"), "input", True, "P1", "Text/numeric input field background (TMP_InputField)."),
    ("input_caret", ("normal",), "input", False, "P1", "Text caret (TMP)."),
    ("tabs", ("active", "inactive", "hover"), "tabs", True, "P1", "Tab header buttons for tabbed panels."),
    ("scroll_track", ("normal",), "scroll", True, "P1", "Scrollbar track."),
    ("scroll_handle", ("normal", "hover"), "scroll", True, "P1", "Scrollbar handle/thumb."),
    ("scroll_mask", ("normal",), "scroll", False, "P1", "Scroll viewport mask (often a plain rounded rect)."),
    ("separator", ("normal",), "panel", True, "P1", "Horizontal separator line."),
    ("progress_bg", ("normal",), "progress", True, "P0", "Progress bar background."),
    ("progress_fill", ("normal",), "progress", True, "P0", "Progress bar fill (Image.filled, pivot left)."),
    ("graph_grid", ("normal",), "graph", True, "P1", "Graph grid line/border."),
    ("graph_axis", ("normal",), "graph", True, "P1", "Graph axis line."),
    ("graph_legend_swatch", ("normal",), "graph", False, "P1", "Colour swatch used in graph legends."),
    ("graph_point", ("normal",), "graph", False, "P2", "Draggable graph point handle (spline editors)."),
    ("graph_bar_fill", ("normal",), "graph", True, "P1", "Gene-analysis bar fill."),
    ("chart_ring_bg", ("normal",), "chart", False, "P1", "Pie/wedge chart background ring."),
    ("chart_ring_segment", ("normal",), "chart", False, "P1", "Pie/wedge segment (may be a generated mesh; extract if sprite-based)."),
    ("icon_star_locked", ("normal",), "icon", False, "P1", "Locked challenge star."),
    ("icon_star_unlocked", ("normal",), "icon", False, "P1", "Unlocked challenge star (tinted by code)."),
    ("icon_checkmark", ("normal",), "icon", False, "P1", "Checkmark (selected states)."),
    ("icon_arrow_up", ("normal",), "icon", False, "P1", "Up/sort arrow."),
    ("icon_arrow_down", ("normal",), "icon", False, "P1", "Down/expand arrow."),
    ("icon_arrow_left", ("normal",), "icon", False, "P2", "Left navigation arrow."),
    ("icon_arrow_right", ("normal",), "icon", False, "P2", "Right navigation arrow."),
    ("icon_warning", ("normal",), "icon", False, "P1", "Warning triangle for dialogs/indicators."),
    ("icon_error", ("normal",), "icon", False, "P1", "Error icon for error popups."),
    ("icon_info", ("normal",), "icon", False, "P1", "Info icon (tooltips/settings)."),
    ("icon_external_link", ("normal",), "icon", False, "P3", "External-link icon (URL buttons)."),
]:
    usage_dir = {
        "button": "button",
        "toggle": "toggle",
        "slider": "slider",
        "dropdown": "dropdown",
        "input": "input",
        "tabs": "tabs",
        "scroll": "scroll",
        "progress": "progress",
        "graph": "graph",
        "chart": "chart",
        "icon": "icon",
        "panel": "panel",
    }[usage]
    logical_name = name[5:] if usage == "icon" and name.startswith("icon_") else name
    E(
        f"ui/{usage_dir}/{logical_name}",
        usage,
        prio,
        variants=variants,
        nine_slice=nine,
        pivot=(0.0, 0.5) if usage in {"slider", "progress"} and "fill" in name else None,
        notes=note,
    )

# -- HUD / in-world UI-ish art ---------------------------------------------
HUD = [
    ("hud/speed_slider_knob", "slider", "P0", ("W2",), "log speed slider knob (reuses slider_knob style)"),
    ("hud/pause_icon", "icon", "P0", ("W2",), "pause glyph (play/pause button)"),
    ("hud/play_icon", "icon", "P0", ("W2",), "play glyph"),
    ("hud/step_icon", "icon", "P0", ("W2",), "single-step glyph (viewer-specific; game uses play/pause)"),
    ("hud/statusbar_frame", "panel", "P0", ("W7",), "floating status-bar frame (bubble)"),
    ("hud/statusbar_health_fill", "progress", "P0", ("W7",), "health fill (green->red tinted by code)"),
    ("hud/statusbar_energy_fill", "progress", "P0", ("W7",), "energy fill (colour tinted by code)"),
    ("hud/time_panel_bg", "panel", "P0", ("W1", "W2"), "backing plate for time/TPS + speed controls"),
    ("hud/visibility_toggle_icons", "toggle", "P0", ("W6",), "pheromone/fertility/ruler/bars/FOV/grid toggle icons"),
    ("hud/rightmost_tabs", "tabs", "P0", ("W4",), "settings/statistics/lineage side-panel tab buttons"),
    ("overlay/selection_rect", "overlay", "P0", ("W8",), "rubber-band selection rectangle (1px tinted border)"),
    ("overlay/selection_highlight_material", "material", "P0", ("W8", "O3"), "highlight shader/material for selected entities (not a PNG)"),
    ("overlay/fov_target_indicator", "icon", "P1", ("O5",), "seen-entity indicator (uses procedural bibite/pellet/egg sprites)"),    ("overlay/ruler", "overlay", "P1", ("O6",), "ruler ticks/labels prefab art"),
    ("overlay/autopan_target", "icon", "P2", ("W12",), "auto-pan target marker"),
]
for logical, usage, prio, surfaces, note in HUD:
    E(
        f"ui/{logical}",
        usage,
        prio,
        surfaces=surfaces,
        extraction="runtime_procedural" if logical == "overlay/fov_target_indicator" else None,
        prefab_only=logical != "overlay/fov_target_indicator",
        notes=note,
    )

E(
    "ui/material/color_spectrum",
    "material",
    "P1",
    surfaces=("E9",),
    extraction="shader_material",
    sources=(src("UIScripts/ColorKillerPanel.cs", 53, "colorSpectrum", "code-assign"),),
    prefab_only=True,
    notes="Colour-killer hue spectrum bar material (shader exposes `_hue`/`_width`); export material or document colours. NOT PRESENT in the 2026-10-30 manifest — needs shader/colour documentation.",
)
E(
    "ui/material/tolerance_band",
    "material",
    "P1",
    surfaces=("E9",),
    extraction="shader_material",
    sources=(src("UIScripts/ColorKillerPanel.cs", 54, "toleranceMat", "code-assign"),),
    prefab_only=True,
    notes="Colour-killer tolerance band material.",
)
for logical, prio, surfaces, aliases, note in [
    ("ui/material/ui_egg", "P1", ("B6",), ["UIEggMat"], "UI egg preview material (Shader Graphs/UIEggShader)."),
    ("ui/material/node_activation", "P1", ("B4", "B5"), ["nodeMaterial"], "Brain node colouring material (Shader Graphs/nodeActivation)."),
    ("ui/material/synapse_activation", "P1", ("B4", "B5"), ["SynapseMaterial"], "Brain synapse colouring material."),
    ("ui/material/lineage_graph", "P2", ("A5",), ["LineageGraphMaterial"], "Lineage tree mesh material (Lineage Mesh Shader)."),
    ("ui/material/stomach_acid", "P0b", ("B3",), ["acidLiquid"], "Biology-panel stomach acid material (UI/StomachAcid)."),
    ("ui/material/internals_preview", "P0b", ("B3",), ["internals_preview", "internalssections"], "Biology-panel organ schematic preview materials."),
    ("ui/material/fov_cone", "P1", ("O5",), ["FieldOfViewMat", "FieldOfViewShader"], "Selected-bibite FOV cone material (Shader Graphs/FieldOfViewShader)."),
]:
    E(
        logical,
        "material",
        prio,
        surfaces=surfaces,
        extraction="shader_material",
        prefab_only=True,
        notes=note,
    )
E(
    "ui/cursor/cursor_edit",
    "cursor",
    "P2",
    surfaces=("E10",),
    sources=(src("UIScripts/UIPrefabsHolder.cs", 32, "editCursor", "code-assign"),),
    notes="Placement/edit cursor texture.",
)
E(
    "ui/cursor/cursor_add",
    "cursor",
    "P2",
    surfaces=("E10",),
    sources=(src("UIScripts/UIPrefabsHolder.cs", 34, "addCursor", "code-assign"),),
    notes="Add-object cursor texture.",
)
E(
    "ui/cursor/cursor_remove",
    "cursor",
    "P2",
    surfaces=("E10",),
    sources=(src("UIScripts/UIPrefabsHolder.cs", 36, "removeCursor", "code-assign"),),
    notes="Remove-object cursor texture.",
)

# -- lineage connectors -----------------------------------------------------
for logical, field, line in [
    ("ui/lineage/elbow_left", "elbowLeft", 44),
    ("ui/lineage/elbow_right", "elbowRight", 44),
    ("ui/lineage/junction_left", "junctionLeft", 45),
    ("ui/lineage/junction_right", "junctionRight", 45),
]:
    E(
        logical,
        "graph",
        "P2",
        surfaces=("A5",),
        sources=(
            src("UIScripts.UIReferences.LineagePanel/LineageTreePanel.cs", None, field, "code-decl"),
            src("UIScripts.UIReferences.LineagePanel/LineageLinkToParentNode.cs", line, "junctionLeft/junctionRight", "code-assign"),
        ),
        notes="Lineage tree connector sprite (assigned from LineageTreePanel fields at runtime).",
    )

# -- menu / shell art -------------------------------------------------------
for logical, prio, surfaces, note in [
    ("ui/menu/title_logo", "P3", ("M1",), "The Bibites title/logo art"),
    ("ui/menu/title_background", "P3", ("M1",), "title splash background"),
    ("ui/menu/menu_background", "P2", ("M2",), "main menu background"),
    ("ui/menu/loading_bar_bg", "P3", ("M11",), "loading progress background"),
    ("ui/menu/loading_bar_fill", "P3", ("M11",), "loading progress fill"),
    ("ui/menu/version_plate", "P3", ("M1", "M2"), "version text plate"),
]:
    E(
        logical,
        "menu",
        prio,
        surfaces=surfaces,
        optional=logical in {"ui/menu/title_logo", "ui/menu/title_background"},
        notes=note
        + (" OPTIONAL: title art may live in scene/level bundles outside the UI sprite batch; a screenshot fallback is acceptable."
           if logical in {"ui/menu/title_logo", "ui/menu/title_background"} else ""),
    )
E(
    "ui/menu/steam_avatar_placeholder",
    "icon",
    "P3",
    surfaces=("M2",),
    sources=(src("ManagementScripts/MenuInitializer.cs", 88, "steamUserIcon", "code-assign"),),
    notes="Steam avatar placeholder (runtime avatar comes from Steam); menu shell only.",
)

# -- achievement icons (Steam/menu, non-goal but extraction is cheap) -------
ACHIEVEMENTS = [
    "ACH_BIBITE_EDITOR", "ACH_BIBITE_EDITOR_100", "ACH_BIBITE_EINSTEIN",
    "ACH_SIM_7DAYS", "ACH_SIM_1FPS", "ACH_BIBITE_1DAY",
    "ACH_BIBITE_100CHILDREN", "ACH_USER_SNAP",
    "ACH_APOCALYPSE_1STAR", "ACH_APOCALYPSE_2STAR", "ACH_APOCALYPSE_3STAR",
    "ACH_LIKERABBITS_1STAR", "ACH_LIKERABBITS_2STAR", "ACH_LIKERABBITS_3STAR",
    "ACH_DOWNWITHTHEBASICS_1STAR", "ACH_DOWNWITHTHEBASICS_2STAR", "ACH_DOWNWITHTHEBASICS_3STAR",
    "ACH_THEULTIMATEDUEL_1STAR", "ACH_THEULTIMATEDUEL_2STAR", "ACH_THEULTIMATEDUEL_3STAR",
]
# Achievements the shipped game has no full icon set for (AchievementManager logs
# this at startup; the 2026-10-30 extraction confirms the sprites are absent).
OPTIONAL_ACHIEVEMENTS = {
    "ACH_BIBITE_100CHILDREN", "ACH_BIBITE_1DAY", "ACH_BIBITE_EDITOR_100",
    "ACH_BIBITE_EINSTEIN", "ACH_SIM_1FPS", "ACH_SIM_7DAYS", "ACH_USER_SNAP",
}
for ach in ACHIEVEMENTS:
    optional = ach in OPTIONAL_ACHIEVEMENTS
    for suffix in ("0", "1"):
        E(
            f"ui/icon/achievement/{ach}_{suffix}",
            "icon",
            "P3",
            surfaces=("M10", "A8"),
            optional=optional,
            sources=(
                src("SteamIntegrations/AchievementManager.cs", None, "achievementIcons", "code-decl"),
                src("SteamIntegrations/Achievement.cs", 12, "spriteAchieved/spriteNotAchieved", "code-decl"),
                src("UIScripts.UIReferences/AchievementItem.cs", 37, "image", "code-assign"),
            ),
            notes=f"Achievement icon {ach} {'unlocked' if suffix == '1' else 'locked'} (sprite asset name `<id>_{suffix}`)."
            + (" OPTIONAL: the game ships no icon for this achievement." if optional else ""),
        )
E(
    "ui/icon/achievement/secret",
    "icon",
    "P3",
    surfaces=("M10",),
    sources=(src("UIScripts.UIReferences/AchievementItem.cs", 17, "secretIcon", "code-decl"),),
    notes="Secret achievement icon.",
)
for logical, field in [
    ("ui/icon/achievement/placeholder_locked", "placeholderLocked"),
    ("ui/icon/achievement/placeholder_unlocked", "placeholderUnlocked"),
]:
    E(
        logical,
        "icon",
        "P3",
        surfaces=("M10",),
        sources=(src("SteamIntegrations/AchievementManager.cs", None, field, "code-decl"),),
        notes="Fallback icon when an achievement has no full icon set.",
    )

# -- fonts ------------------------------------------------------------------
for logical, note in [
    ("ui/font/tmp_regular", "Body/UI font used by TextMeshProUGUI in all panels (TMP font asset → TTF or atlas)."),
    ("ui/font/tmp_bold", "Bold/heading TMP font asset if separate (extract what exists)."),
    ("ui/font/tmp_sdf_atlas", "TMP SDF atlas/material if TTF conversion loses the game's exact glyph look."),
]:
    E(
        logical,
        "font",
        "P0",
        extraction="font_asset",
        sources=(src("(prefab) TextMeshProUGUI references across Here/UIScripts/**", None, None, "prefab"),),
        notes=note,
    )

# -- procedural runtime previews (no PNG extraction) ------------------------
PROC = [
    ("body", "RequestBodySprite", "BibiteGenePreviewer.cs"),
    ("mouth", "RequestMouthSprite", "BibiteGenePreviewer.cs"),
    ("exoskeleton", "RequestExoskeletonSprite", "BibiteGenePreviewer.cs"),
    ("eye", "RequestEyeSprite", "BibiteGenePreviewer.cs"),
    ("arm", "RequestArmSprite", "BibiteGenePreviewer.cs"),
    ("plant", "RequestPlantSprite", "BibiteGenePreviewer.cs"),
    ("meat", "RequestMeatSprite", "BibiteGenePreviewer.cs"),
    ("pellet", "RequestPelletSpriteOfMaterial", "StomachContentPellet.cs"),
    ("egg", "RequestEggSprite", "UIEgg.cs"),
]
for part, request, file in PROC:
    E(
        f"ui/preview/procedural_{part}",
        "preview",
        "P0b",
        extraction="runtime_procedural",
        prefab_only=False,
        sources=(
            src(f"Here/UIScripts/**/{request}", None, request, "code-assign"),
            src("Here/ManagementScripts/ProceduralSpriteManager.cs", None, request, "runtime-generator"),
        ),
        notes=f"Generated at runtime from `parts` TextAsset + material colours; no PNG needed. Requires task-76 to extract the parts TextAsset/material defaults.",
    )

# ---------------------------------------------------------------------------
# Scan the decompiled C#
# ---------------------------------------------------------------------------
UI_GLOBS = [
    "UIScripts/**/*.cs",
    "UIScripts.UIPanels/**/*.cs",
    "UIScripts.UIReferences/**/*.cs",
    "UIScripts.UIReferences.Graphs/**/*.cs",
    "UIScripts.UIReferences.LineagePanel/**/*.cs",
    "UIScripts.SettingHandles/**/*.cs",
    "UIScripts.SettingHandles.References/**/*.cs",
    "UIScripts.SettingHandles.SettingDisplays/**/*.cs",
    "UIScripts.InfoHandles/**/*.cs",
    "UIScripts.CustomGraphics/**/*.cs",
    "SteamIntegrations/*.cs",
    "SimulationScripts.Rendering/*.cs",
]
EXTRA_FILES = [
    "ManagementScripts/GUIManager.cs",
    "ManagementScripts/UserControl.cs",
    "ManagementScripts/CameraManager.cs",
    "ManagementScripts/TimeController.cs",
    "ManagementScripts/TooltipSystem.cs",
    "ManagementScripts/RightmostPanelsManager.cs",
    "ManagementScripts/MenuInitializer.cs",
    "OneUseScripts/BackgroundManager.cs",
]

FIELD_RE = re.compile(
    r"\b(Image|SpriteRenderer|Sprite|TextMeshProUGUI|TMP_FontAsset)\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:=|;)"
)
SPRITE_ASSIGN_RE = re.compile(r"([A-Za-z_][A-Za-z0-9_\.]*)\s*\.\s*sprite\s*=\s*(.+)$")
MATERIAL_ASSIGN_RE = re.compile(r"([A-Za-z_][A-Za-z0-9_\.]*)\s*\.\s*material\s*=\s*(.+)$")
BRAIN_ICON_RE = re.compile(r"BrainIconHolder\.instance")
PROC_RE = re.compile(r"Request(Body|Mouth|Exoskeleton|Eye|Arm|Plant|Meat|Pellet|Egg)Sprite")


def ui_source_files() -> list[Path]:
    files: list[Path] = []
    for pattern in UI_GLOBS:
        files.extend(sorted(HERE.glob(pattern)))
    files.extend(HERE / p for p in EXTRA_FILES if (HERE / p).exists())
    seen = set()
    unique = []
    for f in files:
        if f not in seen:
            seen.add(f)
            unique.append(f)
    return unique


def panel_slug(file_name: str) -> str:
    """Infer a logical panel slug from a UI file name for heuristic naming."""
    stem = re.sub(r"\.cs$", "", file_name)
    overrides = {
        "BibiteStatsPanel": "bibite_stats",
        "GenesPanel": "bibite_genes",
        "BiologyPanel": "bibite_biology",
        "BrainPanel": "bibite_brain",
        "ExpandedBrainPanel": "bibite_brain_expanded",
        "ColorKillerPanel": "tool_panel",
        "RadTowerPanel": "tool_panel",
        "PheroTowerPanel": "tool_panel",
        "SelectionPanel": "selection_multi",
        "EscapePanel": "escape_menu",
        "InformationPanel": "stats_full",
        "GraphsPanel": "graphs",
        "LineageTreePanel": "lineage",
        "LineageWindow": "lineage",
        "BibiteEditorPanel": "genome_editor",
        "BibiteBrainEditor": "brain_editor",
        "LoadGamePanel": "load_game",
        "SaveGamePanel": "save_game",
        "ScenarioSelectorPanel": "scenario_editor",
        "ZonesEditorPanel": "zones_editor",
        "SettingsChangersEditor": "scenario_editor",
        "TemplateSelectorPanel": "template_browser",
        "BibiteTemplateSelectorPanel": "template_browser",
        "VisibilityPanel": "hud_ecology",
        "StatusBarsReference": "hud_statusbars",
        "TooltipTrigger": "tooltip",
        "TooltipHandle": "tooltip",
        "AchievementPanel": "main_menu",
        "ChallengeTrackerPanel": "challenge",
        "UserSettingsManager": "main_menu",
        "ScenariosEditor": "scenario_editor",
        "DynamicSettingsPanel": "dynamic_settings",
        "DefaultBibitesPanel": "scenario_editor",
        "BibiteItem": "template_browser",
        "BibiteGenePreviewer": "genome_editor",
        "BibiteTemplateGenePreviewer": "template_browser",
        "ProceduralGenePreviewer": "genome_editor",
        "SpeciesInfoPanel": "species_info",
        "SpeciesGridPanel": "species_info",
        "SpeciesPanel": "species_info",
    }
    if stem in overrides:
        return overrides[stem]
    return re.sub(r"(?<!^)(?=[A-Z])", "_", stem).lower().strip("_")


def role_from_field(field: str) -> tuple[str, str] | None:
    """Return (usage, name-suffix) heuristic for an Image/Sprite field."""
    f = field.lower()
    if any(k in f for k in ("preview", "rawimage", "rendertexture", "rendertex", "rt")):
        return None
    if any(k in f for k in ("healthbar", "energybar", "fill", "progress")):
        return ("progress", "fill")
    if "slider" in f:
        return ("slider", "track")
    if "star" in f:
        return ("icon", "star")
    if any(k in f for k in ("color", "colour", "spectrum", "tolerance", "gradient")):
        return ("glow", "gradient")
    if any(k in f for k in ("icon", "ico")):
        return ("icon", "icon")
    if any(k in f for k in ("arrow", "chevron", "expand", "collapse")):
        return ("icon", "arrow")
    if any(k in f for k in ("toggle", "check")):
        return ("toggle", "toggle")
    if any(k in f for k in ("background", "bg", "frame", "panel", "window", "container", "holder", "plate")):
        return ("panel", "frame")
    if any(k in f for k in ("mask",)):
        return ("panel", "mask")
    return ("panel", "element")


def scan() -> tuple[list[dict], dict]:
    fields: list[dict] = []
    refs = {
        "brain_icons": [],
        "procedural": [],
        "sprite_assign": [],
        "material_assign": [],
        "icon_holder": [],
    }
    for path in ui_source_files():
        rel = path.relative_to(HERE).as_posix()
        try:
            lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            continue
        for i, line in enumerate(lines, start=1):
            for m in FIELD_RE.finditer(line):
                ftype, name = m.group(1), m.group(2)
                if ftype in {"Image", "SpriteRenderer", "Sprite", "TextMeshProUGUI", "TMP_FontAsset"}:
                    fields.append(
                        {
                            "file": f"Here/{rel}",
                            "line": i,
                            "type": ftype,
                            "field": name,
                            "decl": line.strip(),
                        }
                    )
            if BRAIN_ICON_RE.search(line):
                refs["brain_icons"].append({"file": f"Here/{rel}", "line": i, "ref": line.strip()})
            for m in PROC_RE.finditer(line):
                refs["procedural"].append(
                    {"file": f"Here/{rel}", "line": i, "ref": line.strip(), "request": m.group(0)}
                )
            am = SPRITE_ASSIGN_RE.search(line)
            if am:
                refs["sprite_assign"].append(
                    {"file": f"Here/{rel}", "line": i, "target": am.group(1), "ref": line.strip()}
                )
            mm = MATERIAL_ASSIGN_RE.search(line)
            if mm:
                refs["material_assign"].append(
                    {"file": f"Here/{rel}", "line": i, "target": mm.group(1), "ref": line.strip()}
                )
            if "highlightMaterial" in line:
                refs["icon_holder"].append({"file": f"Here/{rel}", "line": i, "ref": line.strip()})
    # Attach scan evidence to matching curated entries.
    by_logical = {e["logical"]: e for e in ENTRIES}
    for logical, e in by_logical.items():
        if "brain_icon" in logical:
            e["prefab_only"] = False
            for r in refs["brain_icons"]:
                e["source"].append(src(r["file"], r["line"], None, "code-decl/assign"))
        if "procedural_" in logical:
            part = logical.rsplit("_", 1)[-1]
            e["prefab_only"] = False
            for r in refs["procedural"]:
                if part.lower() in r["request"].lower():
                    e["source"].append(src(r["file"], r["line"], None, "code-assign"))
        if logical == "ui/overlay/selection_highlight_material":
            for r in refs["icon_holder"]:
                e["source"].append(src(r["file"], r["line"], "highlightMaterial", "code-decl/assign"))
    # Field inventory with heuristic expectations.
    inventory: list[dict] = []
    for f in fields:
        slug = panel_slug(Path(f["file"]).name)
        role = role_from_field(f["field"])
        if f["type"] in {"TextMeshProUGUI", "TMP_FontAsset"}:
            expected = "ui/font/tmp_regular"
            usage = "font"
            confidence = "prefab"
        elif role is None:
            expected = None
            usage = "runtime"
            confidence = "runtime"
        else:
            usage, suffix = role
            expected = f"ui/{usage}/{slug}_{f['field'].lower()}_{suffix}"
            confidence = "heuristic"
        inventory.append(
            {
                "file": f["file"],
                "line": f["line"],
                "type": f["type"],
                "field": f["field"],
                "panel": slug,
                "usage": usage,
                "expected_logical": expected,
                "confidence": confidence,
                "prefab_only": confidence != "runtime",
            }
        )
    return inventory, refs


# ---------------------------------------------------------------------------
# Brain icons (33 inputs + 13 activation functions) from BrainIconHolder
# ---------------------------------------------------------------------------
INPUT_NAMES = [
    "EnergyRatio", "Maturity", "LifeRatio", "Fullness", "Speed", "RotationSpeed",
    "IsGrabbing", "AttackedDamage", "EggStored", "BibiteCloseness", "BibiteAngle",
    "NBibites", "PlantCloseness", "PlantAngle", "NPlants", "MeatCloseness",
    "MeatAngle", "NMeats", "RedBibite", "GreenBibite", "BlueBibite", "Tic",
    "Minute", "TimeAlive", "PheroSense1", "PheroSense2", "PheroSense3",
    "Phero1Angle", "Phero2Angle", "Phero3Angle", "Phero1Heading", "Phero2Heading",
    "Phero3Heading",
]
FUNCTION_NAMES = [
    "Sigmoid", "Linear", "TanH", "Sine", "ReLu", "Gaussian", "Latch",
    "Differential", "Abs", "Mult", "Integrator", "Inhibitory", "SoftLatch",
]

# Real extracted sprite names (assets/game/manifest.json, 2026-10-30 snapshot).
# Input/output/activation icon names follow the game's own naming; the rest are
# shared chrome sprites reused by many prefabs (many logical panels map to the
# same 9-slice texture). "inferred" aliases are documented in docs/ASSETS_UI.md.
INPUT_ALIASES = [
    "input_energy_ratio", "input_maturity", "input_life_ratio", "input_fullness",
    "input_speed", "input_rotation_speed", "input_is_grabbing_objects",
    "input_attacked_damage", "input_eggstored", "input_bb_conc_weight",
    "input_bb_conc_angle", "input_n_visible_bb", "input_plant_conc_weight",
    "input_plant_conc_angle", "input_n_visible_plant", "input_meat_conc_weight",
    "input_meat_conc_angle", "input_n_visible_meat", "input_closest_bb_r",
    "input_closest_bb_g", "input_closest_bb_b", "input_tic", "input_minute",
    "input_time_alive", "input_pherosense1", "input_pherosense2",
    "input_pherosense3", "input_phero1_angle", "input_phero2_angle",
    "input_phero3_angle", "input_phero1_heading", "input_phero2_heading",
    "input_phero3_heading",
]
OUTPUT_NAMES = [
    "Accelerate", "Rotate", "Herding", "EggProduction", "Want2Lay", "Want2Eat",
    "Digestion", "Grab", "ClkReset", "PhereOut1", "PhereOut2", "PhereOut3",
    "Want2Grow", "Want2Heal", "Want2Attack",
]
OUTPUT_ALIASES = [
    "output_accelerate", "output_rotate", "output_herding", "output_eggProduction",
    "output_want2lay", "output_want2eat", "output_digestion", "output_grab",
    "output_clk_reset", "output_pher_out1", "output_pher_out2", "output_pher_out3",
    "output_want2grow", "output_want2heal", "output_want2attack",
]
FUNCTION_ALIASES = [
    "hidden_sigmoid", "hidden_linear", "hidden_tanh", "hidden_sine", "hidden_relu",
    "hidden_gaussian", "hidden_latch", "hidden_diff", "hidden_abs", "hidden_mult",
    "integrator", "hidden_adaptor2", "hidden_softlatch",
]

# logical -> expected manifest sprite name(s); exact when the name is known,
# inferred/shared otherwise (chrome is reused across panels in the game prefabs).
ALIASES: dict[str, list[str]] = {
    # shared chrome (exact names extracted from sharedassets0..3)
    "ui/button/button": ["Button_UI"],
    "ui/button/button_small": ["Button_UI", "Long_UI"],
    "ui/button/button_icon": ["base_icon", "base_icon_thin"],
    "ui/button/close_x": ["X_icon"],
    "ui/toggle/toggle": ["Checkmark", "Default-Checker-Gray"],
    "ui/toggle/toggle_icon": ["Checkmark", "base_circle_icon"],
    "ui/slider/slider_track": ["bars", "tiled_bars_1"],
    "ui/slider/slider_fill": ["tiled_bars", "tiled_bars_full"],
    "ui/slider/slider_knob": ["Circle", "rond"],
    "ui/dropdown/dropdown_bg": ["Background"],
    "ui/dropdown/dropdown_arrow": ["DropdownArrow"],
    "ui/dropdown/dropdown_item": ["Background", "UISprite"],
    "ui/input/input_bg": ["InputFieldBackground"],
    "ui/input/input_caret": ["Line"],
    "ui/tabs/tabs": ["Button_UI", "UISprite"],
    "ui/scroll/scroll_track": ["scroll"],
    "ui/scroll/scroll_handle": ["rond", "Circle"],
    "ui/scroll/scroll_mask": ["UIMask"],
    "ui/panel/separator": ["Line"],
    "ui/progress/progress_bg": ["tiled_bars_empty"],
    "ui/progress/progress_fill": ["tiled_bars_full", "tiled_bars"],
    "ui/graph/graph_grid": ["Line"],
    "ui/graph/graph_axis": ["Line"],
    "ui/graph/graph_legend_swatch": ["Square"],
    "ui/graph/graph_point": ["Circle"],
    "ui/graph/graph_bar_fill": ["smallGreen", "Square"],
    "ui/chart/chart_ring_bg": ["rond"],
    "ui/chart/chart_ring_segment": ["Circle"],
    "ui/icon/star_locked": ["star_0", "star"],
    "ui/icon/star_unlocked": ["star"],
    "ui/icon/checkmark": ["Checkmark", "checkmark_icon"],
    "ui/icon/arrow_up": ["Arrow"],
    "ui/icon/arrow_down": ["Arrow"],
    "ui/icon/arrow_left": ["Arrow"],
    "ui/icon/arrow_right": ["goto_arrow", "Arrow"],
    "ui/icon/warning": ["information_icon"],
    "ui/icon/error": ["bug_icon", "X_icon"],
    "ui/icon/info": ["information_icon"],
    "ui/icon/external_link": ["gotolink_icon"],
    "ui/cursor/cursor_edit": ["edit cursor"],
    "ui/cursor/cursor_add": ["add cursor"],
    "ui/cursor/cursor_remove": ["remove cursor"],
    "ui/lineage/elbow_left": ["lineage-elbow-left"],
    "ui/lineage/elbow_right": ["lineage-elbow-right"],
    "ui/lineage/junction_left": ["lineage-junction-left"],
    "ui/lineage/junction_right": ["lineage-junction-right"],
    "ui/material/tolerance_band": ["tolerance"],
    "ui/material/color_spectrum": ["color_spectrum", "rainbowMaterial"],
    "ui/material/fov_cone": ["FieldOfViewMat", "field_of_view", "FieldOfViewShader"],
    "ui/overlay/selection_highlight_material": ["highlight", "highlightableGraph"],
    "ui/material/ui_egg": ["UIEggMat", "ui_egg"],
    "ui/material/node_activation": ["nodeMaterial", "node_activation"],
    "ui/material/synapse_activation": ["SynapseMaterial", "synapse_activation"],
    "ui/material/lineage_graph": ["LineageGraphMaterial", "lineage_graph"],
    "ui/material/stomach_acid": ["acidLiquid", "stomach_acid"],
    "ui/material/internals_preview": ["internals_preview", "internalssections"],
    # HUD
    "ui/hud/speed_slider_knob": ["Circle", "rond", "slider_knob"],
    "ui/hud/pause_icon": ["pause_icon"],
    "ui/hud/play_icon": ["resume_icon"],
    "ui/hud/step_icon": ["step_icon"],
    "ui/hud/statusbar_frame": ["tiled_bars_1", "gray_panel", "UISprite"],
    "ui/hud/statusbar_health_fill": ["health_filled"],
    "ui/hud/statusbar_energy_fill": ["energy_filled"],
    "ui/hud/time_panel_bg": ["UISprite", "Long_UI"],
    "ui/hud/rightmost_tabs": ["stats_icon", "settings_icon", "tree_icon"],
    "ui/hud/visibility_toggle_icons": ["eye_icon", "compass", "mesh_icon", "simulation_icon", "stats_icon"],
    "ui/overlay/selection_rect": ["Line"],
    "ui/overlay/autopan_target": ["compass"],
    "ui/overlay/ruler": ["bars"],
    # menu/shell
    "ui/menu/menu_background": ["Wallpaper-white-21", "Background"],
    "ui/menu/loading_bar_bg": ["tiled_bars_empty"],
    "ui/menu/loading_bar_fill": ["tiled_bars_full"],
    "ui/menu/version_plate": ["UISprite"],
    "ui/menu/steam_avatar_placeholder": ["rockstar_placeholder_grayscale"],
    "ui/icon/achievement/secret": ["secret_Placeholder"],
    "ui/icon/achievement/placeholder_locked": ["rockstar_placeholder_grayscale"],
    "ui/icon/achievement/placeholder_unlocked": ["rockstar_placeholder_colored"],
    # fonts
    "ui/font/tmp_regular": ["Roboto-Regular", "Roboto-Regular SDF Atlas", "LiberationSans"],
    "ui/font/tmp_bold": ["Roboto-Bold", "Roboto-Bold SDF Atlas", "Roboto-Bold Atlas"],
    "ui/font/tmp_sdf_atlas": ["Roboto-Regular SDF Atlas", "Roboto-Bold SDF Atlas"],
}
# Panels reuse shared 9-slice chrome; specific textures are per-widget, not per-panel.
PANEL_FRAME_ALIASES = ["UISprite", "Background", "Long_UI", "gray_panel", "gray_panel2"]
PANEL_HEADER_ALIASES = ["gray_panel_head2", "UISprite", "Long_UI"]
for idx, name in enumerate(INPUT_NAMES):
    E(
        f"ui/icon/brain_input/{idx:02d}",
        "icon",
        "P0b",
        surfaces=("B4", "B5", "M16"),
        extraction="sprite_png",
        prefab_only=False,
        sources=(
            src("Here/BrainIconHolder.cs", 7, "brainIcons", "code-decl"),
            src("UIScripts/AddNodeItem.cs", 37, "img", "code-assign"),
            src("UIScripts/UINeuron.cs", 234, "icon", "code-assign"),
        ),
        notes=f"Brain input icon {idx:02d} = {name} (BrainIconHolder.brainIcons[{idx}]).",
    )
for idx, (name, alias) in enumerate(zip(OUTPUT_NAMES, OUTPUT_ALIASES)):
    E(
        f"ui/icon/brain_output/{idx:02d}",
        "icon",
        "P0b",
        surfaces=("B4", "B5", "M16"),
        extraction="sprite_png",
        prefab_only=False,
        sources=(
            src("Here/BrainIconHolder.cs", 7, "brainIcons", "code-decl"),
            src("UIScripts/AddNodeItem.cs", 37, "img", "code-assign"),
        ),
        notes=f"Brain output icon {idx:02d} = {name} (BrainIconHolder.brainIcons[{33 + idx}]).",
    )
for idx, name in enumerate(FUNCTION_NAMES):
    E(
        f"ui/icon/brain_function/{idx:02d}",
        "icon",
        "P0b",
        surfaces=("B4", "B5", "M16"),
        extraction="sprite_png",
        prefab_only=False,
        sources=(
            src("Here/BrainIconHolder.cs", 8, "brainFunctionsIcons", "code-decl"),
            src("UIScripts/AddNodeItem.cs", 41, "img", "code-assign"),
            src("UIScripts/NodeEditor.cs", 304, "icon", "code-assign"),
        ),
        notes=f"Activation-function icon {idx:02d} = {name} (BrainIconHolder.brainFunctionsIcons[{idx}]).",
    )

# ---------------------------------------------------------------------------
# Emit
# ---------------------------------------------------------------------------
def resolve_aliases(entry: dict) -> list[str]:
    """Expected manifest sprite/font names for a logical expectation."""
    logical = entry["logical"]
    if logical in ALIASES:
        return list(ALIASES[logical])
    usage = entry["usage"]
    if usage == "panel" and logical.endswith("_frame"):
        return list(PANEL_FRAME_ALIASES)
    if usage == "panel" and logical.endswith("_header"):
        return list(PANEL_HEADER_ALIASES)
    m = re.match(r"ui/icon/brain_input/(\d+)$", logical)
    if m:
        idx = int(m.group(1))
        if idx < len(INPUT_ALIASES):
            return [INPUT_ALIASES[idx]]
    m = re.match(r"ui/icon/brain_output/(\d+)$", logical)
    if m:
        idx = int(m.group(1))
        if idx < len(OUTPUT_ALIASES):
            return [OUTPUT_ALIASES[idx]]
    m = re.match(r"ui/icon/brain_function/(\d+)$", logical)
    if m:
        idx = int(m.group(1))
        if idx < len(FUNCTION_ALIASES):
            return [FUNCTION_ALIASES[idx]]
    if logical.startswith("ui/icon/achievement/ACH_"):
        # Achievement sprite assets are named exactly after the achievement id.
        name = logical.rsplit("/", 1)[-1]
        return [name]
    if logical.startswith("ui/tooltip/") or logical.startswith("ui/popup/") or logical.startswith("ui/notification/"):
        return ["UISprite", "gray_panel", "Long_UI"]
    return []


def build() -> dict:
    inventory, refs = scan()
    for entry in ENTRIES:
        entry.setdefault("manifest_aliases", resolve_aliases(entry))
    entries = sorted(ENTRIES, key=lambda e: e["id"])
    counts = {"entries": len(entries), "by_priority": {}, "by_usage": {}, "by_extraction": {}}
    for e in entries:
        counts["by_priority"][e["priority"]] = counts["by_priority"].get(e["priority"], 0) + 1
        counts["by_usage"][e["usage"]] = counts["by_usage"].get(e["usage"], 0) + 1
        counts["by_extraction"][e["extraction"]] = counts["by_extraction"].get(e["extraction"], 0) + 1
    mapped = sum(1 for f in inventory if f["expected_logical"])
    counts["field_inventory"] = len(inventory)
    counts["field_inventory_mapped"] = mapped
    counts["field_inventory_prefab_only"] = sum(1 for f in inventory if f["prefab_only"])
    counts["nine_slice_required"] = sum(1 for e in entries if e["nine_slice"])
    counts["entries_with_manifest_aliases"] = sum(1 for e in entries if e.get("manifest_aliases"))
    counts["manifest_alias_count"] = sum(len(e.get("manifest_aliases", [])) for e in entries)
    return {
        "schema_version": 1,
        "generated_by": "tools/assets/build_ui_expectations.py",
        "sources": [
            "docs/design/UI_PARITY.md",
            "Here/UIScripts*/**",
            "Here/ManagementScripts (UI controllers)",
            "Here/SteamIntegrations (menu UI)",
            "Here/SimulationScripts.Rendering (UI-ish overlays)",
            "Here/OneUseScripts/BackgroundManager.cs",
        ],
        "naming_scheme": NAMING,
        "counts": counts,
        "entries": entries,
        "field_inventory": inventory,
        "scan_evidence": {
            "brain_icon_references": len(refs["brain_icons"]),
            "procedural_sprite_requests": len(refs["procedural"]),
            "sprite_assignments": len(refs["sprite_assign"]),
            "material_assignments": len(refs["material_assign"]),
        },
        "prefab_only_note": (
            "All UI chrome (panels/buttons/sliders/toggles/dropdowns/icons/fonts) is wired through "
            "Unity prefabs in the level scenes; the decompiled C# exposes field names but not the "
            "sprite assets. Entries with prefab_only=true must be resolved by extracting the "
            "referenced prefabs (level0..level4 / globalgamemanagers) and matching the field/role. "
            "field_inventory lists every such field with a heuristic expected_logical; treat those "
            "heuristics as a starting point, not ground truth."
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="print the summary without writing")
    parser.add_argument("--output", type=Path, default=OUT, help="output JSON path")
    args = parser.parse_args()
    data = build()
    c = data["counts"]
    print(f"entries: {c['entries']}  (by priority: {c['by_priority']})")
    print(f"usages: {c['by_usage']}")
    print(f"field inventory: {c['field_inventory']} ({c['field_inventory_mapped']} mapped, "
          f"{c['field_inventory_prefab_only']} prefab-only)")
    print(f"9-slice required entries: {c['nine_slice_required']}")
    if args.check:
        return 0
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(data, indent=2, sort_keys=False) + "\n", encoding="utf-8")
    print(f"wrote {args.output.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
