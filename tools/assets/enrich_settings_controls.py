import gzip
import json
from pathlib import Path

root = Path(r"Z:\RUSTibites")
out = root / "assets/game/reference/functional_ui"
d = json.loads((out / "settings_controls.json").read_text(encoding="utf-8"))
runtime = json.loads(gzip.open(root / "assets/game/ui_runtime.json.gz", "rt", encoding="utf-8").read())


def nodes(dump):
    res = []

    def walk(n):
        res.append(n)
        for k in n.get("children") or []:
            walk(k)

    for r in dump.get("canvases") or []:
        walk(r)
    return res


def details(dump):
    texts, buttons = [], []
    for n in nodes(dump):
        if not n.get("active_in_hierarchy"):
            continue
        for c in n.get("components") or []:
            if c.get("type") == "TMPro.TextMeshProUGUI" and c.get("text"):
                texts.append(str(c["text"]))
            elif c.get("type") == "UnityEngine.UI.Button":
                labels = [str(cc.get("text")) for cc in (n.get("components") or [])
                          if cc.get("type") == "TMPro.TextMeshProUGUI" and cc.get("text")]
                buttons.append({"path": n.get("path"), "name": n.get("name"), "enabled": c.get("enabled"), "labels": labels})
    return texts, buttons


for page, sess in [("menu_graphics", "settings_page_graphics_controls"),
                   ("menu_gameplay", "settings_page_gameplay_controls"),
                   ("menu_hotkeys", "settings_page_hotkeys_controls"),
                   ("world_dynamic_parameters", "world_dynamic_parameters_controls"),
                   ("world_settings", "world_settings_controls")]:
    dump = runtime["sessions"].get(sess) or {}
    texts, buttons = details(dump)
    d["pages"][page]["active_texts"] = texts[:250]
    d["pages"][page]["active_buttons"] = buttons[:250]
    # nearest reset control paths (same page) as effect helpers
    d["pages"][page]["reset_paths"] = [b["path"] for b in buttons if "reset" in ((b.get("name") or "") + " " + " ".join(b.get("labels") or [])).lower()][:30]
d["note"] = "Each active Toggle/Slider/Dropdown/Input includes path/current value/options and label_context; " \
            "active_buttons/texts include hotkey bindings and Reset controls. Hotkeys uses Buttons/input fields rather than Toggle/Slider/Dropdown."
(out / "settings_controls.json").write_text(json.dumps(d, indent=2), encoding="utf-8")
cb = json.loads((out / "control_behavior.json").read_text(encoding="utf-8"))
cb["sections"]["settings_pages_full"] = d["pages"]
(out / "control_behavior.json").write_text(json.dumps(cb, indent=2), encoding="utf-8")
m = json.loads((out / "manifest.json").read_text(encoding="utf-8"))
m["settings_controls"] = "assets/game/reference/functional_ui/settings_controls.json"
(out / "manifest.json").write_text(json.dumps(m, indent=2), encoding="utf-8")
r = out / "README.md"
r.write_text(r.read_text(encoding="utf-8") + "\n## Full Settings pages\n\n`settings_controls.json` has all active controls "
             "per page with path/current/options/label context plus active buttons/texts (hotkey rows use buttons). "
             "Sessions `settings_page_{graphics,gameplay,hotkeys}_controls`, `world_{dynamic_parameters,settings}_controls`.\n", encoding="utf-8")
print({k: (len(v.get("controls") or []), len(v.get("active_buttons") or []), len(v.get("active_texts") or [])) for k, v in d["pages"].items()})
