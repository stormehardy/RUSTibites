"""(Texture variant: also `sample_b`, `discard_nz`, `log`, `exp`, several constant buffers and all the inputs.)
Translate the disassembly of a small D3D pixel program (tools/unity/shader_asm.py output) into straight-line Rust.

python dxbc_to_rust.py <asm file> <rust function name> > out.rs

The programs the Biology panel uses (UI/InternalsSections, UI/InternalsPreview, UI/StomachAcid) are plain ALU code: add / mul / mad / div /
dp2 / sqrt / sincos / round / compares / movc / if-else, with no texture reads.  Running the game's own program per pixel on the CPU
reproduces its picture exactly, with nothing to re-derive by hand.  Every register component is a u32 holding either a float's bits or an
integer, the way the GPU keeps it (a compare yields 0xFFFFFFFF / 0).
"""
import re
import sys

asm = open(sys.argv[1], encoding='latin-1').read().splitlines()
name = sys.argv[2]

COMP = {'x': 0, 'y': 1, 'z': 2, 'w': 3}


def parse_operand(text):
    text = text.strip()
    neg = abs_ = False
    if text.startswith('-'):
        neg = True
        text = text[1:]
    if text.startswith('|') and text.endswith('|'):
        abs_ = True
        text = text[1:-1]
    if text == 'null':
        return {'kind': 'null'}
    if text.startswith('l('):
        vals = [v.strip() for v in text[2:-1].split(',')]
        return {'kind': 'lit', 'vals': vals, 'neg': neg, 'abs': abs_}
    m = re.match(r'(r|o|v|cb|t|s)(\d+)(?:\[(\d+)\])?(?:\.([xyzw]+))?$', text)
    if not m:
        raise SystemExit('cannot parse operand ' + text)
    kind, idx, sub, sw = m.groups()
    return {'kind': kind, 'index': int(idx), 'sub': int(sub) if sub is not None else None, 'sw': sw or 'xyzw', 'neg': neg, 'abs': abs_}


def lit_bits(v):
    """The bits of one immediate: an integer literal is an integer, anything with a point or INF is a float."""
    if v.lower().startswith("0x"):
        return int(v, 16)
    if '#INF' in v:
        return 0xFF800000 if v.startswith('-') else 0x7F800000
    if '.' in v or 'e' in v.lower():
        import struct
        return struct.unpack('<I', struct.pack('<f', float(v)))[0]
    return int(v) & 0xFFFFFFFF


def src_bits(op, comp):
    """Rust expression (u32) of the operand's component `comp` (0..3, the destination component's index)."""
    if op['kind'] == 'lit':
        vals = op['vals']
        v = vals[comp] if len(vals) > 1 else vals[0]
        return 'bits(0x%08X)' % lit_bits(v) if False else '0x%08Xu32' % lit_bits(v)
    sw = op['sw']
    c = COMP[sw[comp]] if len(sw) > 1 else COMP[sw[0]]
    if op['kind'] == 'r':
        return 'r[%d][%d]' % (op['index'], c)
    if op['kind'] == 'v':
        return 'v%d[%d]' % (op['index'], c)
    if op['kind'] == 'cb':
        return 'cb%d[%d][%d]' % (op['index'], op['sub'], c)
    raise SystemExit('bad source ' + str(op))


def src_raw(op, comp):
    """The operand's bits with its negate / absolute-value modifiers applied (the sign bit of a float), for the instructions that move or mask
    bits (movc, and, xor): `movc r1.z, r1.z, -r1.w, r1.w` selects the NEGATED value, and `src_bits` alone would drop the minus."""
    e = src_bits(op, comp)
    if op.get('abs'):
        e = '(%s & 0x7FFFFFFFu32)' % e
    if op.get('neg'):
        e = '(%s ^ 0x80000000u32)' % e
    return e


def src_f(op, comp):
    e = 'f(%s)' % src_bits(op, comp)
    if op['abs']:
        e = '%s.abs()' % e
    if op['neg']:
        e = '(-%s)' % e
    return e


def dst(op, comp):
    return 'r[%d][%d]' % (op['index'], comp) if op['kind'] == 'r' else 'o[%d]' % comp


def dest_comps(op):
    return [COMP[ch] for ch in op['sw']]


out = []
indent = 1


def emit(line):
    out.append('    ' * indent + line)


def sat(expr, is_sat):
    return 'sat(%s)' % expr if is_sat else expr


FLOAT_BIN = {'add': '({a} + {b})', 'mul': '({a} * {b})', 'div': '({a} / {b})', 'max': 'fmax({a}, {b})', 'min': 'fmin({a}, {b})'}
FLOAT_UN = {'log': '{a}.log2()', 'exp': '{a}.exp2()', 'sqrt': '{a}.sqrt()', 'rsq': '(1.0 / {a}.sqrt())', 'round_ne': '{a}.round_ties_even()', 'round_ni': '{a}.floor()',
            'frc': '({a} - {a}.floor())', 'mov': '{a}'}
CMP = {'lt': '<', 'ge': '>=', 'eq': '==', 'ne': '!='}

started = False
for line in asm:
    line = line.strip().strip(chr(0)).strip()
    if line.startswith('//') or not line:
        continue
    if line.startswith(('ps_', 'dcl_', 'vs_')):
        continue
    if line == 'ret':
        emit('return Some([f(o[0]), f(o[1]), f(o[2]), f(o[3])]);')
        continue
    if line == 'loop':
        emit('loop {')
        indent += 1
        continue
    if line == 'endloop':
        indent -= 1
        emit('}')
        continue
    if line == 'else':
        indent -= 1
        emit('} else {')
        indent += 1
        continue
    if line == 'endif':
        indent -= 1
        emit('}')
        continue
    m = re.match(r'(\w+)\s+(.*)$', line)
    if not m:
        raise SystemExit('unparsed: ' + repr(line))
    op_name, rest = m.group(1), m.group(2)
    if op_name == 'if_nz':
        a = parse_operand(rest)
        emit('if %s != 0 {' % src_bits(a, 0))
        indent += 1
        continue
    # split operands on commas that are not inside l( ) or [ ]
    parts, depth, cur = [], 0, ''
    for ch in rest:
        if ch in '([':
            depth += 1
        if ch in ')]':
            depth -= 1
        if ch == ',' and depth == 0:
            parts.append(cur)
            cur = ''
        else:
            cur += ch
    parts.append(cur)
    ops = [parse_operand(p) for p in parts]
    if op_name == 'breakc_nz':
        emit('if %s != 0 { break; }' % src_bits(ops[0], 0))
        continue
    if op_name == 'imul' and ops[0]['kind'] == 'null':
        ops = ops[1:]
    base = op_name[:-4] if op_name.endswith('_sat') else op_name
    is_sat = op_name.endswith('_sat')
    if base == 'sincos':
        s, c, a = ops
        comp = None
        if s['kind'] != 'null':
            for ci in dest_comps(s):
                emit('%s = b(f(%s).sin());' % (dst(s, ci), src_bits(a, ci)))
        if c['kind'] != 'null':
            for ci in dest_comps(c):
                emit('%s = b(f(%s).cos());' % (dst(c, ci), src_bits(a, ci)))
        continue
    if base in ('sample_b', 'sample'):
        dstop, uv, tex = ops[0], ops[1], ops[2]
        emit('let texel = tex([%s, %s]);' % (src_f(uv, 0), src_f(uv, 1)))
        for k, ci in enumerate(dest_comps(dstop)):
            emit('%s = b(texel[%d]);' % (dst(dstop, ci), COMP[tex['sw'][k]]))
        continue
    if base == 'discard_nz':
        emit('if %s != 0 { return None; }' % src_bits(ops[0], 0))
        continue
    d = ops[0]
    srcs = ops[1:]
    comps = dest_comps(d)
    # sources are read before any write of this instruction (the dest may be a source): compute into temps first
    stmts = []
    for ci in comps:
        if base in FLOAT_BIN:
            e = FLOAT_BIN[base].format(a=src_f(srcs[0], ci), b=src_f(srcs[1], ci))
            e = 'b(%s)' % sat(e, is_sat)
        elif base in FLOAT_UN:
            e = 'b(%s)' % sat(FLOAT_UN[base].format(a=src_f(srcs[0], ci)), is_sat)
        elif base == 'mad':
            e = 'b(%s)' % sat('(%s * %s + %s)' % (src_f(srcs[0], ci), src_f(srcs[1], ci), src_f(srcs[2], ci)), is_sat)
        elif base == 'div_sat' or (base == 'div' and is_sat):
            e = 'b(%s)' % sat('(%s / %s)' % (src_f(srcs[0], ci), src_f(srcs[1], ci)), True)
        elif base == 'dp2':
            e = 'b(%s)' % sat('(%s * %s + %s * %s)' % (src_f(srcs[0], 0), src_f(srcs[1], 0), src_f(srcs[0], 1), src_f(srcs[1], 1)), is_sat)
            # dp2 swizzles are applied per source component (x, y of the swizzled operand)
            def pick(op, k):
                if op['kind'] == 'lit':
                    return src_f(op, k)
                o2 = dict(op)
                sw = op['sw']
                o2['sw'] = sw[k] if len(sw) > 1 else sw[0]
                return src_f(o2, 0)
            e = 'b(%s)' % sat('(%s * %s + %s * %s)' % (pick(srcs[0], 0), pick(srcs[1], 0), pick(srcs[0], 1), pick(srcs[1], 1)), is_sat)
        elif base == 'dp3':
            # the three-component dot (`Shader Graphs/algeaBloom` normalises the direction to the camera with it); swizzles apply per source component
            def pick3(op, k):
                if op['kind'] == 'lit':
                    return src_f(op, k)
                o2 = dict(op)
                sw = op['sw']
                o2['sw'] = sw[k] if len(sw) > 1 else sw[0]
                return src_f(o2, 0)
            e = 'b(%s)' % sat('(%s * %s + %s * %s + %s * %s)' % (pick3(srcs[0], 0), pick3(srcs[1], 0), pick3(srcs[0], 1), pick3(srcs[1], 1), pick3(srcs[0], 2), pick3(srcs[1], 2)), is_sat)
        elif base in CMP:
            e = 'if %s %s %s { 0xFFFFFFFF } else { 0 }' % (src_f(srcs[0], ci), CMP[base], src_f(srcs[1], ci))
        elif base == 'movc':
            e = 'if %s != 0 { %s } else { %s }' % (src_raw(srcs[0], ci), src_raw(srcs[1], ci), src_raw(srcs[2], ci))
        elif base == 'and':
            e = '(%s & %s)' % (src_raw(srcs[0], ci), src_raw(srcs[1], ci))
        elif base == 'iadd':
            e = '(%s as i32).wrapping_add(%s as i32) as u32' % (src_bits(srcs[0], ci), src_bits(srcs[1], ci))
            if srcs[0].get('neg'):
                e = '(0i32.wrapping_sub(%s as i32)).wrapping_add(%s as i32) as u32' % (src_bits(srcs[0], ci), src_bits(srcs[1], ci))
            elif srcs[1].get('neg'):
                e = '(%s as i32).wrapping_sub(%s as i32) as u32' % (src_bits(srcs[0], ci), src_bits(srcs[1], ci))
        elif base == 'ilt':
            e = 'if (%s as i32) < (%s as i32) { 0xFFFFFFFF } else { 0 }' % (src_bits(srcs[0], ci), src_bits(srcs[1], ci))
        elif base == 'ftoi':
            e = '(%s as i32) as u32' % src_f(srcs[0], ci)
        elif base == 'xor':
            e = '(%s ^ %s)' % (src_raw(srcs[0], ci), src_raw(srcs[1], ci))
        elif base == 'ushr':
            e = '(%s >> (%s & 31))' % (src_bits(srcs[0], ci), src_bits(srcs[1], ci))
        elif base == 'ishl':
            e = '(%s << (%s & 31))' % (src_bits(srcs[0], ci), src_bits(srcs[1], ci))
        elif base == 'imul':
            e = '(%s as i32).wrapping_mul(%s as i32) as u32' % (src_bits(srcs[0], ci), src_bits(srcs[1], ci))
        elif base == 'utof':
            e = 'b(%s as f32)' % src_bits(srcs[0], ci)
        elif base == 'itof':
            e = 'b((%s as i32) as f32)' % src_bits(srcs[0], ci)
        else:
            raise SystemExit('unhandled instruction: ' + line)
        stmts.append((ci, e))
    if len(stmts) == 1:
        emit('%s = %s;' % (dst(d, stmts[0][0]), stmts[0][1]))
    else:
        emit('let t = [%s];' % ', '.join('{ %s }' % e for _, e in stmts))
        for k, (ci, _) in enumerate(stmts):
            emit('%s = t[%d];' % (dst(d, ci), k))

print('/// Generated by tools/unity/dxbc_to_rust.py from the game\'s compiled pixel program; do not edit.')
print('#[allow(clippy::all, unused_assignments, unused_mut, unused_variables, clippy::excessive_precision)]')
with_v3 = len(sys.argv) > 3 and sys.argv[3] in ('v3', 'v3xyz')
v3_width = 3 if len(sys.argv) > 3 and sys.argv[3] == 'v3xyz' else 2
print('pub fn %s(v1: [f32; 2], v2: [f32; 4], %scb0_in: &[[f32; 4]], cb1_in: &[[f32; 4]], tex: &dyn Fn([f32; 2]) -> [f32; 4]) -> Option<[f32; 4]> {' % (name, 'v3: [f32; %d], ' % v3_width if with_v3 else ''))
print('    let v1 = [b(v1[0]), b(v1[1])];')
print('    let v2 = [b(v2[0]), b(v2[1]), b(v2[2]), b(v2[3])];')
if with_v3:
    print('    let v3 = [%s];' % ', '.join('b(v3[%d])' % k for k in range(v3_width)))
print('    let mut cb0 = [[0u32; 4]; 131];')
print('    for (slot, q) in cb0.iter_mut().zip(cb0_in) { *slot = [b(q[0]), b(q[1]), b(q[2]), b(q[3])]; }')
print('    let mut cb1 = [[0u32; 4]; 16];')
print('    for (slot, q) in cb1.iter_mut().zip(cb1_in) { *slot = [b(q[0]), b(q[1]), b(q[2]), b(q[3])]; }')
print('    let mut r = [[0u32; 4]; 16];')
print('    let mut o = [0u32; 4];')
for l in out:
    print(l)
print('    Some([f(o[0]), f(o[1]), f(o[2]), f(o[3])])')
print('}')
