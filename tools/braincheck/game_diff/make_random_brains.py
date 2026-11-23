"""make_random_brains.py <in.zip> <out.zip> <seed> [disable_herding|enable_herding]   (HOSTILE=1 in the environment: weights from 1e-3 to 1e20) - copies a save and gives every bibite a RANDOM brain that uses every node type.

The 33 input and 15 output nodes keep their places (and the outputs their types); each bibite gets 14-30 hidden nodes (every one of the 13 non-input types at least once, biases in the game's
ranges), random synapses (input / hidden -> hidden / output, hidden self loops and cycles, weights up to +-50 now and then so the +-1024 clamp is hit), and random saved node states.  The
game decides what it keeps on load, so the test reads the state of tick 0 from the game itself.  With `disable_herding` the scenario setting is switched on (`enable_herding` off; neither leaves the save as it is) and, for `disable_herding`, some synapses end in the
Herding output (the game's `NodeOut != 2` rule, see README.md).
"""
import json
import math
import os
import random
import sys
import zipfile

src, dst, seed = sys.argv[1], sys.argv[2], int(sys.argv[3])
herding_arg = sys.argv[4] if len(sys.argv) > 4 else ""  # "disable_herding" | "enable_herding" | "" (leave the save as it is)
disable_herding = herding_arg == "disable_herding"
rng = random.Random(seed)
N_IN, N_OUT = 33, 15
TYPE_NAMES = ["Input", "Sigmoid", "Linear", "TanH", "Sine", "ReLu", "Gaussian", "Latch", "Differential", "Abs", "Mult", "Integrator", "Inhibitory", "SoftLatch"]


def f32(x):
    import struct
    return struct.unpack("<f", struct.pack("<f", x))[0]


def bias_for(t):
    if t == 10:  # Mult: a multiplier
        return f32(10 ** rng.uniform(-4, 1)) * rng.choice([-1, 1])
    if t == 11:  # Integrator: no bias
        return 0.0
    if t == 12:  # Inhibitory: a rate in [0.1, 1]
        return f32(rng.uniform(0.1, 1.0))
    if t == 13:  # SoftLatch: [0.1, 100]
        return f32(10 ** rng.uniform(-1, 2))
    return f32(rng.uniform(-2, 2))


HOSTILE = os.environ.get("HOSTILE") == "1"  # weights over many orders of magnitude, so the +-1024 clamp, infinities and NaN happen


def weight():
    if HOSTILE:
        return f32(10 ** rng.uniform(-3, 20) * rng.choice([-1, 1]))
    w = rng.uniform(-5, 5) if rng.random() < 0.9 else rng.uniform(-50, 50)
    return f32(w)


def state(t):
    return f32(rng.uniform(-2, 2))


def make_brain(old):
    nodes = old["Nodes"][: N_IN + N_OUT]
    for i, nd in enumerate(nodes):
        if i >= N_IN:
            nd["baseActivation"] = bias_for(nd["Type"]) if nd["Type"] != 0 else 0.0
            nd["Value"], nd["LastInput"], nd["LastOutput"] = state(0), state(0), state(0)
    types = list(range(1, 14)) * 2
    types += [rng.randint(1, 13) for _ in range(rng.randint(0, 4))]
    rng.shuffle(types)
    n_hidden = len(types)
    for h, t in enumerate(types):
        idx = N_IN + N_OUT + h
        v = state(t)
        nodes.append({"Type": t, "baseActivation": bias_for(t), "TypeName": TYPE_NAMES[t], "Index": idx, "Inov": idx + 1, "Desc": f"Hidden{h}",
                      "Value": v, "LastInput": state(t), "LastOutput": v, "archetype": 2})
    total = N_IN + N_OUT + n_hidden
    syn = []

    def add(a, b, w=None, en=True):
        syn.append({"Inov": len(syn) + 1, "NodeIn": a, "NodeOut": b, "Weight": weight() if w is None else w, "En": en})

    hidden = list(range(N_IN + N_OUT, total))
    outputs = list(range(N_IN, N_IN + N_OUT))
    for h in hidden:  # every hidden node has a way in and a way out (the game removes floating ones)
        add(rng.choice(list(range(N_IN)) + [x for x in hidden if x != h]), h)
        add(h, rng.choice(outputs + [x for x in hidden if x != h]))
    for _ in range(rng.randint(30, 90)):
        a = rng.choice(list(range(N_IN)) + hidden)
        b = rng.choice(hidden + outputs)
        add(a, b, en=rng.random() < 0.9)
    for _ in range(rng.randint(0, 3)):  # self loops
        h = rng.choice(hidden)
        add(h, h)
    for _ in range(rng.randint(3, 8)):  # every output gets a few inputs, so the outputs are not idle
        add(rng.randrange(N_IN), rng.choice(outputs))
    if disable_herding:
        for _ in range(3):
            add(rng.choice(list(range(N_IN)) + hidden), N_IN + 2)
    rng.shuffle(syn)  # the saved order is the order the sums are made in
    for i, s in enumerate(syn):
        s["Inov"] = i + 1
    return {"isReady": True, "Nodes": nodes, "Synapses": syn}


zin = zipfile.ZipFile(src)
zout = zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED)
n = 0
for info in zin.infolist():
    data = zin.read(info.filename)
    name = info.filename
    if name.startswith("bibites/") and name.endswith(".bb8"):
        obj = json.loads(data.decode("utf-8-sig"))
        obj["brain"] = make_brain(obj["brain"])
        data = json.dumps(obj).encode("utf-8")
        n += 1
    elif name == "settings.bb8settings" and herding_arg in ("disable_herding", "enable_herding"):
        st = json.loads(data.decode("utf-8-sig"))

        def patch(d):
            if isinstance(d, dict):
                for k in list(d):
                    if k == "disableHerding":
                        if isinstance(d[k], dict):
                            d[k]["Value"] = disable_herding
                        else:
                            d[k] = disable_herding
                    else:
                        patch(d[k])
            elif isinstance(d, list):
                for x in d:
                    patch(x)

        patch(st)
        data = json.dumps(st).encode("utf-8")
    zout.writestr(info, data)
zout.close()
print("bibites with random brains:", n)
