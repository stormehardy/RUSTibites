import gzip
import json
from pathlib import Path

root = Path(r"Z:\RUSTibites")
out = root / "assets/game/reference/functional_ui"
p = out / "challenge_lineage_workshop.json"
d = json.loads(p.read_text(encoding="utf-8"))
rt = json.loads(gzip.open(root / "assets/game/ui_runtime.json.gz", "rt", encoding="utf-8").read())
summary = {}
for sess in d.get("steps", []):
    pass
for sess in json.loads((out / "manifest.json").read_text(encoding="utf-8")).get("challenge_lineage_workshop_sessions", []):
    s = rt["sessions"].get(sess) or {}
    st = s.get("state") or {}
    summary[sess] = {"nodes": s.get("node_count"), "controls": len(st.get("controls") or []),
                     "texts": (st.get("info") or {}).get("texts", [])[:80],
                     "buttons": (st.get("info") or {}).get("buttons", [])[:80],
                     "panels": (st.get("info") or {}).get("panels", [])[:40],
                     "images": (st.get("info") or {}).get("images", [])[:20]}
d["sessions"] = summary
d["availability"] = {
    "challenge_tracker_end_popup": "No challenge tracker/end-popup/challengeend nodes were present in the local menu UI or lineage overlay.",
    "rung_history": "No rung-history nodes were present in the captured UI.",
    "lineage_species": "AncestralLineagePanel opened but the default save has insufficient lineage data; species tabs/info/genes/brain nodes are captured in the session trees (text 'Not enough data collected...').",
    "workshop": "Local Workshop panel and EditSharedItemPanel captured; subscribed/shared lists show placeholders; edit panel has 9 controls.",
}
p.write_text(json.dumps(d, indent=2), encoding="utf-8")
print({k: (v["nodes"], v["controls"], len(v["texts"])) for k, v in summary.items()})
