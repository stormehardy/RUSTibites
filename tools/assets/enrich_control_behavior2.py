import gzip
import json
from pathlib import Path

root = Path(r"Z:\RUSTibites")
ref = root / "assets/game/reference/functional_ui"
d = json.loads((ref / "control_behavior.json").read_text(encoding="utf-8"))
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


def add_session(sess):
    s = runtime["sessions"].get(sess) or {}

    def walk(n):
        p = n.get("path")
        if p:
            context.setdefault(p, []).extend(texts_desc(n))
        for k in n.get("children") or []:
            walk(k)

    for r in s.get("canvases") or []:
        walk(r)


for sess in ("main_menu_settings", "main_world_settings_panel", "main_world_tool_placer",
             "main_world_toolbar", "main_menu_load_game", "main_world_information", "main_menu_functional_title"):
    add_session(sess)


def labels_for(path):
    if not path:
        return []
    out = []
    parts = path.split("/")
    for i in range(len(parts), 0, -1):
        p = "/".join(parts[:i])
        for t in context.get(p, [])[:10]:
            if t not in out:
                out.append(t)
    return out[:8]


for a in d["sections"]["settings_controls"]["actions"]:
    a["label_context"] = labels_for(a.get("path"))
for key in ("load", "save"):
    for c in (d["sections"]["save_load_browsers"][key].get("controls") or []):
        c["label_context"] = labels_for(c.get("path"))
for c in (d["sections"]["placer_properties"].get("controls") or []):
    c["label_context"] = labels_for(c.get("path"))
d["label_enrichment"] = "label_context is collected from ancestor/descendant TMP text in atlas UI sessions."
(ref / "control_behavior.json").write_text(json.dumps(d, indent=2), encoding="utf-8")
print("settings labels", [a["label_context"] for a in d["sections"]["settings_controls"]["actions"]])
print("placer labels", [c["label_context"] for c in d["sections"]["placer_properties"]["controls"]])
print("save labels", [c["label_context"] for c in (d["sections"]["save_load_browsers"]["save"].get("controls") or [])][:8])
