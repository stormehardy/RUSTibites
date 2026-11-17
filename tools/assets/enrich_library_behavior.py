import gzip
import json
from pathlib import Path

root = Path(r"Z:\RUSTibites")
ref = root / "assets/game/reference/functional_ui"
beh = json.loads((ref / "library_newgame_behavior.json").read_text(encoding="utf-8"))
runtime = json.loads(gzip.open(root / "assets/game/ui_runtime.json.gz", "rt", encoding="utf-8").read())
context = {}


def texts_desc(n):
    out = []
    for c in n.get("components") or []:
        if c.get("type") == "TMPro.TextMeshProUGUI" and c.get("text"):
            out.append(str(c["text"]))
    for k in n.get("children") or []:
        out += texts_desc(k)
    return out


def add(sess):
    s = runtime["sessions"].get(sess) or {}

    def walk(n):
        if n.get("path"):
            context.setdefault(n["path"], []).extend(texts_desc(n))
        for k in n.get("children") or []:
            walk(k)
    for r in s.get("canvases") or []:
        walk(r)


for sess in ("library_selector_flow", "library_create_new", "library_editor_genes_flow",
             "library_editor_brain_flow", "newgame_scenario_options", "newgame_new_scenario_button",
             "newgame_new_challenge_button", "scenario_editor_flow", "scenario_editor_zones",
             "scenario_editor_ChallengeOptions", "scenario_editor_sim"):
    add(sess)


def labels_for(path):
    if not path:
        return []
    out = []
    parts = path.split("/")
    for i in range(len(parts), 0, -1):
        for t in context.get("/".join(parts[:i]), [])[:12]:
            if t not in out:
                out.append(t)
    return out[:10]


for a in beh["actions"]:
    for key in ("before", "after", "controls"):
        for c in a.get(key) or []:
            c["label_context"] = labels_for(c.get("path"))
    for b in a.get("active_buttons") or []:
        b["label_context"] = labels_for(b.get("path"))
beh["label_enrichment"] = "label_context from ancestor/descendant TMP text in the captured atlas sessions."
beh["known_paths"] = {
    "library_create_new": "UI/BibiteSelector/SelectionPanel/Holder/Panels/LeftSide Buttons/CreateNew",
    "library_templates": "UI/BibiteSelector/SelectionPanel/Holder/Panels/LeftSide Buttons/Templates",
    "library_edit": "UI/BibiteSelector/SelectionPanel/Holder/Preview/Right Side Buttons/Edit Button",
    "newgame_start": "UI/NewGameScenarios/ScenarioSelector/Holder/RightPanels/RightGroup/ParametersPanel/Content/Bottom/StartButton",
}
(ref / "library_newgame_behavior.json").write_text(json.dumps(beh, indent=2), encoding="utf-8")
print("newgame labels", [c["label_context"] for a in beh["actions"] if a.get("step") == "newgame_options" for c in a.get("controls") or []])
print("gene labels", [c["label_context"] for a in beh["actions"] if a.get("step") == "editor_genes" for c in (a.get("before") or [])][:10])
