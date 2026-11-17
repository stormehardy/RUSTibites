import json
import sys

path = sys.argv[1] if len(sys.argv) > 1 else r"Z:\RUSTibites\testdata\parity\round2_lol_compare.json"
d = json.load(open(path, encoding="utf-8"))
print("overall", d["overall"], "exit", d["exit_code"], "failures", d["failures"], "warnings", d["warnings"])
m = d["meta"]
print("settings_match", m["settings_match"], "sim_tps", m["sim_tps_left"], m["sim_tps_right"],
      "common", m["common_ticks"], "only", m["only_left_n"], m["only_right_n"])
print("gaps", json.dumps(m.get("gaps"))[:400])
c = d["systems"]["ecology"]
print("curve fails:", [(k, round(v.get("max", 0), 4)) for k, v in c.items() if v["status"] == "fail"])
print("first diffs:", {k: v.get("first_difference_tick") for k, v in c.items() if v.get("first_difference_tick") is not None})
e = d["systems"]["entities"]
print("entities:", {k: f"{v['match']}/{v['n']}" for k, v in e["digests"].items()})
print("energy exact_frac", e["energy"].get("exact_frac"), "pos_mae", round(e["position"].get("mae") or 0, 3),
      "vel_mae", round(e["velocity"].get("mae") or 0, 3), "compared", e["bibites_compared"],
      "snapshot ticks", e["ticks_with_entities"])
bo = [x for x in d["checks"] if x["name"] == "brain_outputs"][0]["detail"]
print("brain values", bo["values_exact"], "/", bo["values_n"], "all_exact", bo["all_exact"],
      "non_exact", bo["non_exact_nodes"][:8])
print("events:", d["systems"]["events"])
