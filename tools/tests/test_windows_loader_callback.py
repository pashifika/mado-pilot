"""Final callback bytes, control-flow boundaries and manifest identity."""

import copy
import hashlib
import importlib.util
from pathlib import Path
import struct
import unittest


SPEC = importlib.util.spec_from_file_location(
    "windows_loader_callback",
    Path(__file__).resolve().parents[1] / "native-release-profile/windows_loader_callback.py",
)
callback = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(callback)


def fixture(tail=b"\xc3"):
    capability = {"protocol": callback.PROTOCOL, "callback": {
        "rva": 0x1000, "statics": {
            "sequence": {"rva": 0x2000, "bytes": 8},
            "incomplete": {"rva": 0x2008, "bytes": 4},
            "records": {"rva": 0x2010, "bytes": 1058816},
        }}}
    code = bytearray()
    for target in (0x2000, 0x2008, 0x2010):
        code += b"\x48\x8d\x05" + struct.pack("<i", target - (0x1000 + len(code) + 7))
    code += tail
    data = bytearray(0xa00)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3c, 0x80)
    data[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HH", data, 0x84, 0x8664, 4)
    struct.pack_into("<H", data, 0x94, 240)
    opt = 0x98
    struct.pack_into("<H", data, opt, 0x20b)
    struct.pack_into("<Q", data, opt + 24, 0x140000000)
    struct.pack_into("<I", data, opt + 108, 16)
    struct.pack_into("<II", data, opt + 136, 0x200000, 12)
    sections = (
        (b".text", 0x200, 0x1000, 0x200, 0x400, 0x60000020),
        (b".data", 0x110000, 0x2000, 0, 0, 0xc0000040),
        (b".pdata", 0x200, 0x200000, 0x200, 0x600, 0x40000040),
        (b".xdata", 0x200, 0x201000, 0x200, 0x800, 0x40000040),
    )
    for index, (name, size, rva, raw_size, raw, flags) in enumerate(sections):
        pos = opt + 240 + index * 40
        data[pos:pos + len(name)] = name
        struct.pack_into("<IIII", data, pos + 8, size, rva, raw_size, raw)
        struct.pack_into("<I", data, pos + 36, flags)
    data[0x400:0x400 + len(code)] = code
    struct.pack_into("<III", data, 0x600, 0x1000, 0x1000 + len(code), 0x201000)
    data[0x800:0x804] = b"\x01\0\0\0"
    return bytes(data), capability, bytes(code)


class CallbackProofTests(unittest.TestCase):
    def test_proof_binds_executable_function_and_static_references(self):
        data, capability, code = fixture()
        proof = callback.inspect_callback(data, capability)
        self.assertEqual(proof["executable_sha256"], hashlib.sha256(data).hexdigest())
        self.assertEqual(proof["callback_sha256"], hashlib.sha256(code).hexdigest())
        self.assertEqual((proof["callback_begin_rva"], proof["callback_end_rva"]), (0x1000, 0x1016))
        self.assertEqual(proof["reachable_instructions"], 4)
        self.assertEqual(proof["reachable_returns"], 1)
        self.assertEqual([row["rip_target"] for row in proof["instructions"]],
                         [0x2000, 0x2008, 0x2010, None])
        changed = bytearray(data)
        changed[0x40] = 1
        other = callback.inspect_callback(bytes(changed), capability)
        self.assertNotEqual(other["executable_sha256"], proof["executable_sha256"])
        self.assertEqual(other["callback_sha256"], proof["callback_sha256"])

    def test_calls_indirect_transfers_unknown_and_unreachable_bytes_refuse(self):
        for tail in (b"\xe8\0\0\0\0", b"\xff\xd0", b"\xff\xe0", b"\x0f\x0b",
                     b"\xc3\xe8\0\0\0\0", b"\x48", b"\x48\x8b"):
            with self.subTest(tail=tail.hex()):
                data, capability, _ = fixture(tail)
                with self.assertRaises(ValueError):
                    callback.inspect_callback(data, capability)

    def test_mixed_push_operand_prefix_cannot_hide_indirect_jump(self):
        data, capability, _ = fixture(bytes.fromhex("66 48 68 00 00 c3 b8 ff e0 00 00 c3"))
        with self.assertRaisesRegex(ValueError, "unsupported-push-size-prefix"):
            callback.inspect_callback(data, capability)

    def test_branch_must_land_on_an_instruction_inside_the_function(self):
        for tail in (b"\xeb\xea", b"\xeb\x00"):
            with self.subTest(tail=tail.hex()):
                data, capability, _ = fixture(tail)
                with self.assertRaisesRegex(ValueError, "branch-outside-function-or-into-instruction"):
                    callback.inspect_callback(data, capability)

    def test_fallthrough_and_returnless_loop_refuse(self):
        for tail, reason in ((b"\x90", "fallthrough-outside-function"),
                             (b"\xeb\xfe", "return-and-all-named-statics-required")):
            with self.subTest(tail=tail.hex()):
                data, capability, _ = fixture(tail)
                with self.assertRaisesRegex(ValueError, reason):
                    callback.inspect_callback(data, capability)

    def test_unknown_static_reference_and_overlapping_layout_refuse(self):
        data, capability, _ = fixture()
        outside = bytearray(data)
        struct.pack_into("<i", outside, 0x403, 0x1000000 - 0x1007)
        with self.assertRaisesRegex(ValueError, "unknown-rip-relative-reference"):
            callback.inspect_callback(bytes(outside), capability)
        capability["callback"]["statics"]["incomplete"]["rva"] = 0x2004
        with self.assertRaisesRegex(ValueError, "nonoverlapping-named-statics"):
            callback.inspect_callback(data, capability)

    def test_capability_rejects_wrong_protocol_boolean_and_changed_layout(self):
        data, original, _ = fixture()
        variants = []
        value = copy.deepcopy(original)
        value["protocol"] = "unreviewed"
        variants.append(value)
        value = copy.deepcopy(original)
        value["callback"]["rva"] = True
        variants.append(value)
        value = copy.deepcopy(original)
        value["callback"]["statics"]["records"]["bytes"] += 1
        variants.append(value)
        value = copy.deepcopy(original)
        value["callback"]["statics"]["sequence"]["rva"] = (1 << 32) - 4
        variants.append(value)
        for value in variants:
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    callback.inspect_callback(data, value)

    def test_exact_unwind_boundary_and_no_handlers_are_required(self):
        data, capability, _ = fixture()
        for offset, payload, reason in (
            (0x600, struct.pack("<I", 0x0fff), "exact-unique-runtime-function"),
            (0x800, b"\x09", "unwind-version-one-no-handler-or-chain"),
            (0x800, b"\x21", "unwind-version-one-no-handler-or-chain"),
            (0x98 + 136, struct.pack("<I", 0xffffffff), "unique-section-range"),
        ):
            with self.subTest(offset=offset, payload=payload.hex()):
                changed = bytearray(data)
                changed[offset:offset + len(payload)] = payload
                with self.assertRaisesRegex(ValueError, reason):
                    callback.inspect_callback(bytes(changed), capability)

    def test_code_must_not_be_writable_and_statics_must_not_be_executable(self):
        data, capability, _ = fixture()
        for index, flags, reason in (
            (0, 0xe0000020, "callback-executable-nonwritable"),
            (1, 0xe0000040, "static-writable-nonexecutable"),
        ):
            with self.subTest(index=index):
                changed = bytearray(data)
                struct.pack_into("<I", changed, 0x98 + 240 + index * 40 + 36, flags)
                with self.assertRaisesRegex(ValueError, reason):
                    callback.inspect_callback(bytes(changed), capability)

    def test_unwind_operations_validate_opcode_register_and_slot_extent(self):
        data, capability, _ = fixture()
        for raw, reason in (
            ("01010100010f0000", "unsupported-unwind-operation"),
            ("0101010001110000", "unwind-operation-slot-bound"),
            ("0101010001000000", "unwind-nonvolatile-register"),
            ("0101010001030000", "unwind-frame-operation"),
        ):
            with self.subTest(raw=raw):
                changed = bytearray(data)
                payload = bytes.fromhex(raw)
                changed[0x800:0x800 + len(payload)] = payload
                with self.assertRaisesRegex(ValueError, reason):
                    callback.inspect_callback(bytes(changed), capability)

    def test_partial_section_overlap_cannot_hide_writable_callback_bytes(self):
        data, capability, _ = fixture()
        changed = bytearray(data)
        struct.pack_into("<H", changed, 0x86, 5)
        pos = 0x98 + 240 + 4 * 40
        struct.pack_into("<IIII", changed, pos + 8, 0x200, 0x1010, 0x200, 0x800)
        struct.pack_into("<I", changed, pos + 36, 0xe0000020)
        with self.assertRaisesRegex(ValueError, "overlapping-section-ranges"):
            callback.inspect_callback(bytes(changed), capability)

    def test_retained_proof_rederives_bytes_not_cached_verdicts(self):
        data, capability, _ = fixture()
        proof = callback.inspect_callback(data, capability)
        self.assertEqual(callback.validate_proof(proof, capability, proof["executable_sha256"]), proof)
        for key, value in (("accepted", 1), ("callback_sha256", "0" * 64),
                           ("reachable_returns", 2), ("unwind_info", "01010100010f0000")):
            with self.subTest(key=key):
                changed = copy.deepcopy(proof)
                changed[key] = value
                with self.assertRaises(ValueError):
                    callback.validate_proof(changed, capability, proof["executable_sha256"])
        changed = copy.deepcopy(proof)
        changed["instructions"][-1]["bytes"] = "ffe0"
        with self.assertRaises(ValueError):
            callback.validate_proof(changed, capability, proof["executable_sha256"])
        with self.assertRaises(ValueError):
            callback.validate_proof(proof, capability, "0" * 64)


if __name__ == "__main__":
    unittest.main()
