"""Protect external-copy isolation and feature admission without running Cargo or native tools."""

import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location(
    "external_rust_consumer", Path(__file__).resolve().parents[1] / "check-external-rust-consumer.py"
)
consumer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(consumer)


class ExternalRustConsumerTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.checkout = self.root / "product"
        self.checkout.mkdir()

    def test_refused_work_roots_preserve_caller_data(self):
        existing = self.root / "existing"
        existing.mkdir()
        sentinel = existing / "caller-data"
        sentinel.write_bytes(b"do not replace")
        with self.assertRaises(ValueError):
            consumer.fresh_root(existing, self.checkout)
        with self.assertRaises(ValueError):
            consumer.fresh_root(self.checkout / "external-looking", self.checkout)
        self.assertEqual(sentinel.read_bytes(), b"do not replace")
        self.assertFalse((self.checkout / "external-looking").exists())

    def test_parent_cargo_config_cannot_leak_into_copy(self):
        parent = self.root / "parent"
        (parent / ".cargo").mkdir(parents=True)
        (parent / ".cargo/config.toml").write_text('[build]\nrustflags = ["--cfg=leaked"]\n', encoding="utf-8")
        destination = parent / "new-proof"
        with self.assertRaises(ValueError):
            consumer.fresh_root(destination, self.checkout)
        self.assertFalse(destination.exists())

    def test_copy_rewrites_only_dependency_line_and_preserves_lock(self):
        self.checkout = self.checkout / "\U00010400"
        self.checkout.mkdir()
        source = self.checkout / "consumer"
        (source / ".cargo").mkdir(parents=True)
        (source / "src").mkdir()
        manifest = (
            b'# mado-pilot = { path = "../../crates/mado-pilot" }\r\n'
            b'[package]\r\nname = "mado-pilot-input-workflow"\r\nedition = "2024"\r\n'
            b'[workspace]\r\nresolver = "3"\r\n[dependencies]\r\n'
            + consumer.DEPENDENCY + b'\r\n'
        )
        (source / "Cargo.toml").write_bytes(manifest)
        (source / "Cargo.lock").write_bytes(b"version = 4\n# consumer-owned lock\n")
        (source / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.97.1"\n', encoding="utf-8")
        (source / ".cargo/config.toml").write_text(
            '[env]\nMACOSX_DEPLOYMENT_TARGET = { value = "26.5.2", force = true }\n', encoding="utf-8"
        )
        (source / "src/main.rs").write_text("fn main() {}\n", encoding="utf-8")
        destination = self.root / "copied consumer"
        consumer.copy_consumer(source, destination, self.checkout)
        absolute = json.dumps((self.checkout / "crates/mado-pilot").as_posix(), ensure_ascii=False)
        expected = manifest.replace(b"\r\n" + consumer.DEPENDENCY + b"\r\n",
                                    f'\r\nmado-pilot = {{ path = {absolute} }}\r\n'.encode())
        self.assertEqual((destination / "Cargo.toml").read_bytes(), expected)
        self.assertEqual((source / "Cargo.toml").read_bytes(), manifest)
        self.assertEqual((destination / "Cargo.lock").read_bytes(), (source / "Cargo.lock").read_bytes())
        (source / "Cargo.lock").write_bytes(b"source changed after copy")
        self.assertEqual((destination / "Cargo.lock").read_bytes(), b"version = 4\n# consumer-owned lock\n")

    def test_cargo_child_ignores_ambient_build_and_model_configuration(self):
        rustup_home = self.root / "rustup-home"
        rustup_home.mkdir()
        inherited = {
            "HOME": str(self.root), "USERPROFILE": str(self.root), "SystemRoot": str(self.root),
            "RUSTUP_HOME": str(rustup_home), "RUSTUP_TOOLCHAIN": "nightly",
            "CARGO_HOME": str(self.checkout / ".cargo"), "CARGO_TARGET_DIR": str(self.checkout / "target"),
            "CARGO_BUILD_TARGET": "wrong-target", "RUSTC_WRAPPER": "sccache", "RUSTFLAGS": "--cfg=leaked",
            "CARGO_ENCODED_RUSTFLAGS": "--cfg=leaked", "MADO_PILOT_ONNX_RUNTIME": "private-runtime",
            "MADO_PILOT_ONNX_DETECTOR": "private-model", "DYLD_LIBRARY_PATH": "ambient-native",
        }
        with mock.patch.dict(os.environ, inherited, clear=True):
            environment = consumer.baseline_environment(self.root / "bin/rustup", self.root / "proof")
        self.assertEqual(environment["RUSTUP_TOOLCHAIN"], "1.97.1")
        self.assertEqual(environment["CARGO_HOME"], str(self.root / "proof/cargo-home"))
        self.assertEqual(environment["CARGO_TARGET_DIR"], str(self.root / "proof/target"))
        forbidden = {"CARGO_BUILD_TARGET", "RUSTC_WRAPPER", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS",
                     "MADO_PILOT_ONNX_RUNTIME", "MADO_PILOT_ONNX_DETECTOR", "DYLD_LIBRARY_PATH"}
        self.assertFalse(forbidden.intersection(environment))

    def test_transitive_private_feature_refuses_public_facade_graph(self):
        facade = self.checkout / "crates/mado-pilot"
        facade.mkdir(parents=True)
        (facade / "Cargo.toml").write_text('[package]\nname = "mado-pilot"\n', encoding="utf-8")
        external = self.root / "consumer"
        external.mkdir()
        target = self.root / "target"
        target.mkdir()
        metadata = {
            "workspace_root": str(external), "target_directory": str(target), "workspace_members": ["app"],
            "packages": [
                {"id": "app", "name": consumer.PACKAGE,
                 "dependencies": [{"name": "mado-pilot", "path": str(facade), "kind": None}]},
                {"id": "facade", "name": "mado-pilot", "manifest_path": str(facade / "Cargo.toml"), "source": None},
            ],
            "resolve": {"root": "app", "nodes": [
                {"id": "app", "features": []}, {"id": "facade", "features": []},
            ]},
        }
        consumer.assert_graph(metadata, external, self.checkout, target)
        metadata["resolve"]["nodes"][1]["features"] = ["native-template-watch-qualification"]
        with self.assertRaisesRegex(ValueError, "private fixture/qualification/benchmark"):
            consumer.assert_graph(metadata, external, self.checkout, target)


if __name__ == "__main__":
    unittest.main()
