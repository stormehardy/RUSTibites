"""Disassemble the DXBC programs of a game shader (read-only; the game install is untouched):
python shader_asm.py <asset file> <path_id> <out prefix>
Literals are printed as hex bits (D3D_DISASM_PRINT_HEX_LITERALS), which `dxbc_to_rust.py` needs. Needs lz4 (a UnityPy dependency) and d3dcompiler_47.dll (Windows)."""
import ctypes
import struct
import sys

import lz4.block
import UnityPy

asset, path_id, out_prefix = sys.argv[1], int(sys.argv[2]), sys.argv[3]
env = UnityPy.load('C:/Users/Storme/TheBibites/The Bibites_Data/' + asset)
shader = next(o.read() for o in env.objects if o.path_id == path_id and o.type.name == 'Shader')
platforms = shader.platforms
blobs = []
compressed = bytes(shader.compressedBlob)
for index, platform in enumerate(platforms):
    for offset, clen, dlen in zip(shader.offsets[index], shader.compressedLengths[index], shader.decompressedLengths[index]):
        blob = lz4.block.decompress(compressed[offset:offset + clen], uncompressed_size=dlen)
        blobs.append((platform, blob))
        print('platform', platform, 'blob', len(blob))

d3d = ctypes.WinDLL('d3dcompiler_47.dll')


class ID3DBlob(ctypes.Structure):
    pass


def disassemble(code: bytes) -> str:
    out = ctypes.c_void_p()
    buffer = ctypes.create_string_buffer(code, len(code))
    d3d.D3DDisassemble.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_uint, ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
    hr = d3d.D3DDisassemble(buffer, len(code), 0x80, None, ctypes.byref(out))
    if hr != 0:
        return f'D3DDisassemble failed: 0x{hr & 0xffffffff:08x}'
    # ID3DBlob vtable: QueryInterface, AddRef, Release, GetBufferPointer, GetBufferSize
    vtable = ctypes.cast(ctypes.cast(out, ctypes.POINTER(ctypes.c_void_p))[0], ctypes.POINTER(ctypes.c_void_p))
    get_ptr = ctypes.WINFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p)(vtable[3])
    get_size = ctypes.WINFUNCTYPE(ctypes.c_size_t, ctypes.c_void_p)(vtable[4])
    pointer = get_ptr(out)
    size = get_size(out)
    return ctypes.string_at(pointer, size).decode('latin-1')


count = 0
for platform, blob in blobs:
    position = 0
    while True:
        position = blob.find(b'DXBC', position)
        if position < 0:
            break
        size = struct.unpack_from('<I', blob, position + 24)[0]
        code = blob[position:position + size]
        text = disassemble(code)
        name = f'{out_prefix}_{count}.asm.txt'
        open(name, 'w', encoding='utf-8').write(text)
        first = next((line for line in text.splitlines() if line.startswith(('vs_', 'ps_'))), '?')
        print(name, size, 'bytes', first)
        count += 1
        position += max(size, 4)
print('programs', count)
