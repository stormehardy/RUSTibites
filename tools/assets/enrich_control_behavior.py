import gzip
import json
from pathlib import Path

root = Path(r"Z:\RUSTibites")
ref = root / "assets/game/reference/functional_ui"
d = json.loads((ref / "control_behavior.json").read_text(encoding="utf-8"))
runtime = json.loads(gzip.open(root / "assets/game/ui_runtime.json.gz", "rt", encoding="utf-8").read())

# build path -> node texts (including descendants) from every session for label context
context = {}


def add_session(sess):
    s = runtime["sessions"].get(sess) or {}

    def walk(n):
        p = n.get("path")
        if p:
            txt = []
            for c in n.get("components") or []:
                if c.get("type") == "TMPro.TextMeshProUGUI" and c.get("text"):
                    txt.append(str(c["text"]))
            context.setdefault(p, []).extend(txt)
        for k in n.get("children") or []:
            walk(k)

    for r in s.get("canvases") or []:
        walk(r)


for sess in ("main_menu_settings", "main_world_settings_panel", "main_world_tool_placer",
             "main_world_toolbar", "main_menu_load_game", "main_world_information"):
    add_session(sess)


def labels_for(path):
    if not path:
        return []
    out = []
    parts = path.split("/")
    for i in range(len(parts), 0, -1):
        p = "/".join(parts[:i])
        for t in context.get(p, [])[:6]:
            if t not in out:
                out.append(t)
    return out[:8]


for a in d["sections"]["settings_controls"]["actions"]:
    a["label_context"] = labels_for(a.get("path"))
for c in (d["sections"]["save_load_browsers"]["load"].get("controls") or []):
    c["label_context"] = labels_for(c.get("path"))
for c in (d["sections"]["save_load_browsers"]["save"].get("controls") or []):
    c["label_context"] = labels_for(c.get("path"))
for c in (d["sections"]["placer_properties"].get("controls") or []):
    c["label_context"] = labels_for(c.get("path"))
for c in (d["sections"]["kill_dialog"].get("active_controls") or []):
    c["label_context"] = labels_for(c.get("path"))
d["label_enrichment"] = "label_context is collected from the nearest ancestor/descendant TMP text in the atlas UI sessions."
(ref / "control_behavior.json").write_text(json.dumps(d, indent=2), encoding="utf-8")
print("settings labels", [a["label_context"] for a in d["sections"]["settings_controls"]["actions"]])
print("placer labels", [c["label_context"] for c in d["sections"]["placer_properties"]["controls"]])
