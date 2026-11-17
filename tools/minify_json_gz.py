"""Strips insignificant whitespace from a (huge, pretty-printed) JSON .gz stream."""
import gzip, sys

src, dst = sys.argv[1], sys.argv[2]
in_str = esc = False
ws = b' \n\r\t'
with gzip.open(src, 'rb') as f, gzip.open(dst, 'wb', compresslevel=6) as out:
    while True:
        chunk = f.read(1 << 24)
        if not chunk:
            break
        res = bytearray()
        for b in chunk:
            if in_str:
                res.append(b)
                if esc:
                    esc = False
                elif b == 0x5C:
                    esc = True
                elif b == 0x22:
                    in_str = False
            elif b == 0x22:
                in_str = True
                res.append(b)
            elif b not in ws:
                res.append(b)
        out.write(bytes(res))
