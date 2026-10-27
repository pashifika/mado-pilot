"""Bounded final-PE callback inspection; source review owns memory and loop semantics."""

import hashlib
import json
import struct

PROTOCOL = "mado-component-windows-loader-v1"
CODE_CAP = 32 * 1024
IMAGE_CAP = 64 * 1024 * 1024
STATIC_SIZES = {"sequence": 8, "incomplete": 4, "records": 1058816}


def require(value, reason):
    if not value:
        raise ValueError(reason)


def u32(value, name):
    require(type(value) is int and 0 <= value < 1 << 32, name)
    return value


def validate_capability(capability):
    require(isinstance(capability, dict) and set(capability) == {"protocol", "callback"}
            and capability["protocol"] == PROTOCOL, "callback-capability-protocol")
    callback = capability["callback"]
    require(isinstance(callback, dict) and set(callback) == {"rva", "statics"},
            "callback-capability-fields")
    u32(callback["rva"], "callback-rva")
    statics = callback["statics"]
    require(isinstance(statics, dict) and set(statics) == set(STATIC_SIZES), "callback-static-set")
    for name, size in STATIC_SIZES.items():
        fact = statics[name]
        require(isinstance(fact, dict) and set(fact) == {"rva", "bytes"}, "callback-static-fields")
        u32(fact["rva"], "callback-static-rva")
        require(type(fact["bytes"]) is int and fact["bytes"] == size
                and fact["rva"] + size <= 1 << 32, "callback-static-layout")
    return callback


def validate_unwind(data, function_size):
    """Validate supported version-one UWOP forms, not stack-instruction equivalence."""
    require(len(data) >= 4 and data[0] == 1, "unwind-version-one-no-handler-or-chain")
    prolog, count, frame = data[1], data[2], data[3]
    require(prolog <= function_size and len(data) == 4 + 2 * ((count + 1) & ~1),
            "unwind-prolog-or-array-bound")
    nonvolatile = {3, 5, 6, 7, 12, 13, 14, 15}
    frame_register = frame & 15
    require(frame_register in nonvolatile if frame_register else frame == 0,
            "unwind-frame-register")
    slot, previous, frame_set = 0, prolog, 0
    while slot < count:
        offset, encoded = data[4 + slot * 2:6 + slot * 2]
        operation, info = encoded & 15, encoded >> 4
        require(0 < offset <= previous, "unwind-code-offset-order")
        previous = offset
        extra = 0
        if operation == 0:
            require(info in nonvolatile, "unwind-nonvolatile-register")
        elif operation == 1:
            require(info in (0, 1), "unwind-allocation-form")
            extra = 1 if info == 0 else 2
        elif operation == 2:
            pass
        elif operation == 3:
            require(info == 0 and frame_register != 0, "unwind-frame-operation")
            frame_set += 1
        elif operation in (4, 5):
            require(info in nonvolatile, "unwind-nonvolatile-register")
            extra = 1 if operation == 4 else 2
        elif operation in (8, 9):
            require(6 <= info <= 15, "unwind-nonvolatile-xmm")
            extra = 1 if operation == 8 else 2
        else:
            raise ValueError("unsupported-unwind-operation")
        require(slot + extra < count, "unwind-operation-slot-bound")
        if extra:
            value = int.from_bytes(data[6 + slot * 2:6 + (slot + extra) * 2], "little")
            if operation == 1:
                require((value > 0 if info == 0 else value > 0 and value % 8 == 0),
                        "unwind-allocation-size")
            elif operation in (5, 9):
                require(value % (8 if operation == 5 else 16) == 0, "unwind-save-offset")
        slot += 1 + extra
    require(frame_set == int(frame_register != 0), "unwind-frame-definition")


class Image:
    def __init__(self, data, file_size=None):
        self.data = data
        self.file_size = len(data) if file_size is None else file_size
        require(type(self.file_size) is int and len(data) <= self.file_size <= IMAGE_CAP,
                "callback-file-size")
        require(len(data) >= 64 and data[:2] == b'MZ', 'dos-header')
        nt = self.u32(0x3c)
        require(nt <= 1 << 20 and nt + 24 <= len(data) and data[nt:nt + 4] == b'PE\0\0', 'pe-header')
        machine, count = struct.unpack_from('<HH', data, nt + 4)
        size = struct.unpack_from('<H', data, nt + 20)[0]
        opt = nt + 24
        require(machine == 0x8664 and 1 <= count <= 96 and size >= 144
                and opt + size + 40 * count <= len(data), 'pe-layout')
        self.header_end = opt + size + 40 * count
        require(struct.unpack_from('<H', data, opt)[0] == 0x20b, 'pe32-plus')
        self.base = struct.unpack_from('<Q', data, opt + 24)[0]
        require(self.u32(opt + 108) >= 4, 'exception-directory-required')
        self.exception_rva, self.exception_size = struct.unpack_from('<II', data, opt + 136)
        self.sections = []
        for index in range(count):
            pos = opt + size + index * 40
            virtual, rva, raw_size, raw = struct.unpack_from('<IIII', data, pos + 8)
            flags = self.u32(pos + 36)
            require(raw + raw_size <= self.file_size, 'section-file-range')
            extent = max(virtual, raw_size)
            require(rva + extent <= 1 << 32, "section-rva-overflow")
            if extent:
                require(all(not max(s["virtual_size"], s["raw_size"])
                            or rva + extent <= s["rva"]
                            or s["rva"] + max(s["virtual_size"], s["raw_size"]) <= rva
                            for s in self.sections), "overlapping-section-ranges")
            self.sections.append({'rva': rva, 'virtual_size': virtual, 'raw_size': raw_size,
                                  'raw': raw, 'flags': flags})

    def u32(self, pos):
        require(0 <= pos <= len(self.data) - 4, 'pe-u32-range')
        return struct.unpack_from('<I', self.data, pos)[0]

    def section(self, rva, size, raw=False):
        require(size > 0 and rva >= 0, 'rva-range')
        found = [s for s in self.sections if s['rva'] <= rva
                 and rva + size <= s['rva'] + (s['raw_size'] if raw else max(s['virtual_size'], s['raw_size']))]
        require(len(found) == 1, 'unique-section-range')
        return found[0]

    def at(self, rva, size):
        section = self.section(rva, size, raw=True)
        pos = section['raw'] + rva - section['rva']
        return self.data[pos:pos + size]

    def function(self, start):
        require(self.exception_size and self.exception_size % 12 == 0
                and self.exception_size <= 4 * 1024 * 1024, 'bounded-pdata')
        rows = list(struct.iter_unpack('<III', self.at(self.exception_rva, self.exception_size)))
        matches = [(begin, end, unwind) for begin, end, unwind in rows if begin <= start < end]
        require(len(matches) == 1 and matches[0][0] == start, 'exact-unique-runtime-function')
        begin, end, unwind = matches[0]
        require(0 < end - begin <= CODE_CAP, 'callback-size-bound')
        require(unwind % 4 == 0, "unwind-alignment")
        header = self.at(unwind, 4)
        validate_unwind(self.at(unwind, 4 + 2 * ((header[2] + 1) & ~1)), end - begin)
        section = self.section(begin, end - begin, raw=True)
        require(section['flags'] & 0x20000000 and not section['flags'] & 0x80000000,
                'callback-executable-nonwritable')
        return end, unwind, self.at(begin, end - begin)


def decode(code, base):
    """Only these x86-64 encodings are admitted; there is no generic fallback.

    Operand bytes are consumed structurally (ModRM/SIB/displacement/immediate).
    The decoder neither executes nor claims to prove memory/loop semantics.
    """
    rows = []
    pos = 0
    while pos < len(code):
        start = pos
        prefixes = []
        while pos < len(code) and code[pos] in (0x66, 0x2e, 0xf0, 0xf2, 0xf3):
            require(pos - start < 14, 'prefix-length-bound')
            prefixes.append(code[pos])
            pos += 1
        require(pos < len(code), 'truncated-prefix')
        rex = 0
        if 0x40 <= code[pos] <= 0x4f:
            rex = code[pos]
            pos += 1
        require(pos < len(code), 'truncated-opcode')
        opcode = code[pos]
        pos += 1
        two = opcode == 0x0f
        if two:
            require(pos < len(code), 'truncated-two-byte-opcode')
            opcode = (opcode << 8) | code[pos]
            pos += 1
        if 0x2e in prefixes or len(set(prefixes)) != len(prefixes):
            # LLVM alignment padding can contain repeated 66 plus CS prefixes.
            # Permit them only on the explicit multi-byte NOP encoding.
            require(opcode == 0x0f1f and all(value in (0x66, 0x2e) for value in prefixes),
                    'non-nop-duplicate-or-segment-prefix')
        width = 8 if rex & 8 else 2 if 0x66 in prefixes else 4
        kind = 'ordinary'
        relative = None
        immediate = 0
        modrm = False
        group = None
        rip_disp = None
        memory = False
        reg = None
        if opcode == 0xc3:
            kind = 'return'
        elif opcode in (0xe9, 0xeb) or 0x70 <= opcode <= 0x7f or 0x0f80 <= opcode <= 0x0f8f:
            kind = 'jump' if opcode in (0xe9, 0xeb) else 'conditional'
            immediate = 4 if opcode == 0xe9 or opcode >= 0x0f80 else 1
            relative = True
        elif 0x50 <= opcode <= 0x5f or opcode in (0x90, 0x98, 0x99):
            pass
        elif 0xb0 <= opcode <= 0xb7:
            immediate = 1
        elif 0xb8 <= opcode <= 0xbf:
            immediate = width
        elif opcode in (0x68, 0x6a):
            require(not (0x66 in prefixes and rex & 8), 'unsupported-push-size-prefix')
            immediate = 1 if opcode == 0x6a else min(width, 4)
        elif opcode in (0x04, 0x0c, 0x24, 0x2c, 0x34, 0x3c, 0xa8):
            immediate = 1
        elif opcode in (0x05, 0x0d, 0x25, 0x2d, 0x35, 0x3d, 0xa9):
            immediate = min(width, 4)
        elif opcode in (0x00, 0x01, 0x02, 0x03, 0x08, 0x09, 0x0a, 0x0b,
                        0x20, 0x21, 0x22, 0x23, 0x28, 0x29, 0x2a, 0x2b,
                        0x30, 0x31, 0x32, 0x33, 0x38, 0x39, 0x3a, 0x3b,
                        0x63, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8d,
                        0x0faf, 0x0fb0, 0x0fb1, 0x0fb6, 0x0fb7, 0x0fbe, 0x0fbf,
                        0x0f10, 0x0f11, 0x0f28, 0x0f29, 0x0f57, 0x0f6f, 0x0f7f, 0x0fef):
            modrm = True
        elif 0x0f40 <= opcode <= 0x0f4f or 0x0f90 <= opcode <= 0x0f9f:
            modrm = True
        elif opcode in (0x69, 0x6b):
            modrm = True
            immediate = 1 if opcode == 0x6b else min(width, 4)
        elif opcode in (0x80, 0x81, 0x83):
            modrm = True
            group = {0, 1, 4, 5, 6, 7}
            immediate = min(width, 4) if opcode == 0x81 else 1
        elif opcode in (0xc6, 0xc7):
            modrm, group = True, {0}
            immediate = min(width, 4) if opcode == 0xc7 else 1
        elif opcode in (0xc0, 0xc1, 0xd0, 0xd1, 0xd2, 0xd3):
            modrm, group = True, {4, 5, 7}
            immediate = 1 if opcode in (0xc0, 0xc1) else 0
        elif opcode in (0xf6, 0xf7):
            modrm, group = True, {0, 2, 3}
        elif opcode in (0xfe, 0xff):
            modrm, group = True, {0, 1}  # FF call/jmp/push are deliberately refused.
        elif opcode == 0x0f1f:
            modrm, group = True, {0}
        else:
            raise ValueError('unknown-pe-opcode-at-' + hex(base + start))
        if modrm:
            require(pos < len(code), 'truncated-modrm')
            value = code[pos]
            pos += 1
            mod, reg, rm = value >> 6, (value >> 3) & 7, value & 7
            memory = mod != 3
            require(group is None or reg in group, 'forbidden-opcode-extension')
            require(opcode != 0x8d or memory, 'lea-memory-required')
            if opcode in (0xf6, 0xf7) and reg == 0:
                immediate = 1 if opcode == 0xf6 else min(width, 4)
            displacement = 0
            if memory:
                if rm == 4:
                    require(pos < len(code), 'truncated-sib')
                    sib = code[pos]
                    pos += 1
                    if mod == 0 and sib & 7 == 5:
                        displacement = 4
                elif mod == 0 and rm == 5:
                    displacement = 4
                    require(pos + 4 <= len(code), 'truncated-rip-displacement')
                    rip_disp = struct.unpack_from('<i', code, pos)[0]
                if mod == 1:
                    displacement = 1
                elif mod == 2:
                    displacement = 4
            pos += displacement
        if opcode == 0x0f1f:
            rip_disp = None  # NOP consumes addressing bytes but accesses no memory.
        if 0xf0 in prefixes:
            require(memory and (opcode in (0x08, 0x09, 0x0fb0, 0x0fb1)
                    or opcode in (0x80, 0x81, 0x83) and reg == 1), 'unsupported-lock-form')
        if 0xf2 in prefixes or 0xf3 in prefixes:
            require(0xf2 not in prefixes and opcode in (0x0f6f, 0x0f7f)
                    and 0x66 not in prefixes and 0xf0 not in prefixes, 'unsupported-repeat-prefix')
        if opcode in (0x0f6f, 0x0f7f, 0x0fef):
            require(0x66 in prefixes or 0xf3 in prefixes and opcode != 0x0fef, 'mmx-forbidden')
        require(pos + immediate <= len(code), 'truncated-instruction')
        target = None
        if relative:
            require(not prefixes and not rex, 'prefixed-branch-refused')
            delta = int.from_bytes(code[pos:pos + immediate], 'little', signed=True)
            target = base + pos + immediate + delta
        pos += immediate
        require(0 < pos - start <= 15, 'instruction-length')
        rows.append({'rva': base + start, 'bytes': code[start:pos].hex(), 'size': pos - start,
                     'opcode': hex(opcode), 'kind': kind, 'target': target,
                     'rip_target': None if rip_disp is None else base + pos + rip_disp})
    return rows

def inspect_callback(data, capability):
    """Inspect supplied executable bytes and actual manifest RVAs without executing them."""
    require(isinstance(data, bytes) and len(data) <= IMAGE_CAP, "callback-image-bound")
    return _inspect_image(Image(data), capability, hashlib.sha256(data).hexdigest())


def _inspect_image(image, capability, executable_sha256):
    callback = validate_capability(capability)
    statics = callback["statics"]
    for name, fact in statics.items():
        section = image.section(fact["rva"], fact["bytes"])
        require(section["flags"] & 0x80000000 and not section["flags"] & 0x20000000,
                "static-writable-nonexecutable")
    names = tuple(statics)
    for index, left in enumerate(names):
        for right in names[index + 1:]:
            a, b = statics[left], statics[right]
            require(a["rva"] + a["bytes"] <= b["rva"] or b["rva"] + b["bytes"] <= a["rva"],
                    "nonoverlapping-named-statics")
    start = callback["rva"]
    finish, unwind, code = image.function(start)
    rows = decode(code, start)
    require(len(rows) <= 20000, "instruction-count-bound")
    by_address = {row["rva"]: row for row in rows}
    seen_statics = set()
    for row in rows:
        if row["target"] is not None:
            require(row["target"] in by_address, "branch-outside-function-or-into-instruction")
        if row["rip_target"] is not None:
            matches = [name for name, fact in statics.items()
                       if fact["rva"] <= row["rip_target"] < fact["rva"] + fact["bytes"]]
            require(len(matches) == 1, "unknown-rip-relative-reference")
            seen_statics.add(matches[0])
    pending, reachable, returns = [start], set(), 0
    while pending:
        address = pending.pop()
        if address in reachable:
            continue
        require(address in by_address, "fallthrough-outside-function")
        reachable.add(address)
        row = by_address[address]
        if row["kind"] == "return":
            returns += 1
            continue
        if row["target"] is not None:
            pending.append(row["target"])
        if row["kind"] != "jump":
            pending.append(address + row["size"])
    require(returns > 0 and seen_statics == set(STATIC_SIZES), "return-and-all-named-statics-required")
    return {"schema": "mado-component-windows-callback-proof-v1", "accepted": True,
            "executable_sha256": executable_sha256, "executable_bytes": image.file_size,
            "capability": capability, "callback_begin_rva": start, "callback_end_rva": finish,
            "unwind_rva": unwind, "callback_sha256": hashlib.sha256(code).hexdigest(),
            "pe_headers": image.data[:image.header_end].hex(),
            "runtime_functions": image.at(image.exception_rva, image.exception_size).hex(),
            "unwind_info": image.at(unwind, 4 + 2 * ((image.at(unwind, 4)[2] + 1) & ~1)).hex(),
            "instructions": rows, "reachable_instructions": len(reachable),
            "all_decoded_instructions": len(rows), "reachable_returns": returns}


def _hex_bytes(value, cap):
    require(isinstance(value, str) and len(value) % 2 == 0 and len(value) <= cap * 2
            and all(c in "0123456789abcdef" for c in value), "callback-proof-hex-bound")
    return bytes.fromhex(value)


class _ProofImage(Image):
    def __init__(self, proof):
        super().__init__(_hex_bytes(proof.get("pe_headers"), 2 * 1024 * 1024),
                         proof.get("executable_bytes"))
        instructions = proof.get("instructions")
        require(isinstance(instructions, list) and 0 < len(instructions) <= 20000
                and all(isinstance(row, dict) for row in instructions), "callback-proof-instructions")
        pieces = [_hex_bytes(row.get("bytes"), 15) for row in instructions]
        require(sum(map(len, pieces)) <= CODE_CAP, "callback-proof-code-bound")
        self.regions = (
            (self.exception_rva, _hex_bytes(proof.get("runtime_functions"), 4 * 1024 * 1024)),
            (u32(proof.get("unwind_rva"), "callback-proof-unwind"), _hex_bytes(proof.get("unwind_info"), 516)),
            (u32(proof.get("callback_begin_rva"), "callback-proof-start"), b"".join(pieces)),
        )

    def at(self, rva, size):
        self.section(rva, size, raw=True)
        found = [(base, data) for base, data in self.regions
                 if base <= rva and rva + size <= base + len(data)]
        require(len(found) == 1, "callback-proof-region")
        base, data = found[0]
        return data[rva - base:rva - base + size]


def validate_proof(proof, capability, executable_sha256):
    """Recheck retained PE fragments; live admission separately inspects the actual EXE."""
    require(isinstance(proof, dict) and isinstance(executable_sha256, str)
            and len(executable_sha256) == 64
            and all(c in "0123456789abcdef" for c in executable_sha256), "callback-proof-identity")
    derived = _inspect_image(_ProofImage(proof), capability, executable_sha256)
    require(json.dumps(proof, sort_keys=True, allow_nan=False)
            == json.dumps(derived, sort_keys=True, allow_nan=False), "callback-proof-content-drift")
    return derived
