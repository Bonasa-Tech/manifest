#!/usr/bin/env python3
"""Check the actual proof ELF, including mutable state and constant permissions."""

import re
import struct
import sys
from pathlib import Path


def check(path, finalize=False):
    data = bytearray(Path(path).read_bytes())
    header = struct.unpack_from('<16sHHIQQQIHHHHHH', data)
    if header[0][:7] != b'\x7fELF\x02\x01\x01' or header[2] != 247 or header[7] != 3:
        raise ValueError('expected a little-endian ELF64 sBPF v3 program')
    sections = [
        struct.unpack_from('<IIQQQQIIQQ', data, header[6] + i * header[11])
        for i in range(header[12])
    ]

    def contents(section):
        return data[section[4]:section[4] + section[5]]

    def string(table, offset):
        return table[offset:table.index(b'\0', offset)].decode()

    names = contents(sections[header[13]])
    by_name = {string(names, section[0]): (i, section) for i, section in enumerate(sections)}
    globals_index, globals_section = by_name['.data']
    rodata_index, rodata = by_name['.rodata']
    if not rodata[2] & 2 or (rodata[2] & 1 and not finalize):
        raise ValueError('.rodata must be allocated and read-only')
    if globals_section[2] & 7 != 3 or not globals_section[5]:
        raise ValueError('.data must be nonempty, writable, allocated and non-executable')

    segments = [
        struct.unpack_from('<IIQQQQQQ', data, header[5] + i * header[9])
        for i in range(header[10])
    ]
    for section, flags in [(rodata, 4), (globals_section, 6)]:
        covering = [
            segment for segment in segments if segment[0] == 1
            and segment[3] <= section[3]
            and section[3] + section[5] <= segment[3] + segment[6]
        ]
        if len(covering) != 1 or covering[0][1] != flags:
            raise ValueError('constants and mutable globals need separate R and RW load segments')

    # Every mutable global in these mocks must survive in the writable section,
    # not become an absolute/discarded symbol or share the constants' section.
    source_dir = Path(__file__).resolve().parent
    expected = set()
    for source in ['hooks.rs', 'summaries/token.rs', '../state/cvt_db_mock.rs', '../state/cvt_global_mock.rs']:
        expected.update(re.findall(r'^static mut (\w+):', (source_dir / source).read_text(), re.MULTILINE))
    symbols = by_name['.symtab'][1]
    strings = contents(sections[symbols[6]])
    retained = set()
    for offset in range(symbols[4], symbols[4] + symbols[5], symbols[9]):
        name, info, _, section_index, address, size = struct.unpack_from('<IBBHQQ', data, offset)
        if info & 15 == 1 and section_index == globals_index and size:
            if not globals_section[3] <= address < address + size <= globals_section[3] + globals_section[5]:
                raise ValueError('global symbol falls outside the writable section')
            retained.add(string(strings, name))
    missing = sorted(name for name in expected if not any(name in symbol for symbol in retained))
    if not expected or missing:
        raise ValueError(f'mutable verification globals missing from .data: {missing}')
    if finalize and rodata[2] & 1:
        # LLVM emits constant pointer tables as .data.rel.ro (SHF_WRITE until
        # relocation). The v3 linker resolves them into a read-only PT_LOAD,
        # but retains that input flag on .rodata. Mirror the segment permission
        # in its section header, only after checking all mutable mock globals
        # are elsewhere. No instructions, addresses or data bytes are changed.
        flags_offset = header[6] + rodata_index * header[11] + 8
        struct.pack_into('<Q', data, flags_offset, rodata[2] & ~1)
        Path(path).write_bytes(data)
        check(path)
        return
    print(f'{path}: sBPF v3, read-only constants, {len(expected)} mutable globals in a separate RW segment')


if __name__ == '__main__':
    if len(sys.argv) != 2:
        sys.exit(f'usage: {sys.argv[0]} <verification.so>')
    try:
        check(sys.argv[1])
    except (ValueError, KeyError, OSError, struct.error, IndexError) as error:
        sys.exit(f'Invalid Certora ELF: {error}')
