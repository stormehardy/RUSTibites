"""Splits assets/game/ui_runtime_native.json.gz (minified) into one gz per session in
assets/game/ui_runtime_native/<name>.json.gz so the viewer loads sessions lazily.
Re-run after regenerating the monolith."""
import gzip, os, re, sys

src = sys.argv[1] if len(sys.argv) > 1 else 'assets/game/ui_runtime_native.json.gz'
out = sys.argv[2] if len(sys.argv) > 2 else 'assets/game/ui_runtime_native'
os.makedirs(out, exist_ok=True)
data = gzip.open(src).read()
pos = data.index(b'"sessions":{') + len(b'"sessions":{')
key = re.compile(rb'"([^"]+)":\{')
names = []
while True:
    m = key.match(data, pos)
    if not m:
        break
    start = m.end() - 1  # the '{'
    depth, instr, esc, p = 0, False, False, start
    while True:
        c = data[p]
        if instr:
            if esc: esc = False
            elif c == 92: esc = True
            elif c == 34: instr = False
        elif c == 34: instr = True
        elif c == 123: depth += 1
        elif c == 125:
            depth -= 1
            if depth == 0:
                p += 1
                break
        p += 1
    name = m.group(1).decode()
    with gzip.open(os.path.join(out, name + '.json.gz'), 'wb', compresslevel=9) as f:
        f.write(data[start:p])
    names.append(name)
    if data[p:p + 1] == b',':
        pos = p + 1
    else:
        break
print(len(names), 'sessions')
