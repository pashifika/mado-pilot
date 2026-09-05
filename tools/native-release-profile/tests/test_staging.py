"""Exercise staging identity, safety and accounting rules with small synthetic files only."""

import hashlib
import os
import stat
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import staging  # noqa: E402

LICENSE_TEXT = b"MadoPilot license text\n"
OPENCV_LICENSE_TEXT = b"Apache License 2.0 text\n"
RUST_CONSUMER = b"#!/bin/sh\nexit 0\n"
CAPI_LIBRARY = b"capi" * 100
OPENCV_LIBRARY = b"opencv" * 50
MODEL = b"m" * 1000
CUDNN = b"c" * 200
C_CONSUMER = b"C" * 50
FIXTURE = b"F" * 60
LIMIT = 1 << 20
PACKAGE_BYTES = (
    len(LICENSE_TEXT) + len(RUST_CONSUMER) + len(CAPI_LIBRARY) + len(OPENCV_LIBRARY) + len(OPENCV_LICENSE_TEXT)
)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def declare(identifier, root, path, data, *, category, ownership, destination=None, notice="license", **overrides):
    row = {
        "id": identifier,
        "root": root,
        "path": path,
        "destination": destination,
        "category": category,
        "ownership": ownership,
        "sha256": sha256(data),
        "bytes": len(data),
        "source": f"{identifier} source",
        "version": "1.0",
        "license": "MIT",
        "notice": notice,
        "redistribution": "host-only" if ownership == "host" else "approved",
    }
    row.update(overrides)
    return row


class StagingTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.roots = {alias: self.base / alias for alias in ("build", "opencv", "host")}
        for root in self.roots.values():
            root.mkdir()
        self.destination = self.base / "destination"
        self.destination.mkdir()

    def write(self, alias, relative, data, mode=None):
        path = self.roots[alias] / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        if mode is not None:
            path.chmod(mode)
        return path

    def bundled(self, identifier, path, data, **overrides):
        overrides.setdefault("destination", path)
        overrides.setdefault("notice", "opencv-license")
        return declare(identifier, "opencv", path, data, category="native_payload", ownership="bundled", **overrides)

    def manifest(self):
        self.write("build", "LICENSE", LICENSE_TEXT)
        self.write("build", "bin/consumer", RUST_CONSUMER, 0o755)
        self.write("build", "lib/libmado_pilot_capi.dylib", CAPI_LIBRARY)
        self.write("build", "bin/c-consumer", C_CONSUMER, 0o755)
        self.write("build", "fixtures/frame.bin", FIXTURE)
        self.write("opencv", "lib/libopencv_core.dylib", OPENCV_LIBRARY)
        self.write("opencv", "LICENSE", OPENCV_LICENSE_TEXT)
        self.write("host", "models/det.onnx", MODEL)
        self.write("host", "cudnn64_8.dll", CUDNN)
        return [
            declare("license", "build", "LICENSE", LICENSE_TEXT, category="notices", ownership="product", destination="LICENSE"),
            declare("consumer-rs", "build", "bin/consumer", RUST_CONSUMER, category="rust_consumer", ownership="product", destination="bin/consumer"),
            declare("capi", "build", "lib/libmado_pilot_capi.dylib", CAPI_LIBRARY, category="shared_library", ownership="product", destination="lib/libmado_pilot_capi.dylib"),
            declare("consumer-c", "build", "bin/c-consumer", C_CONSUMER, category="consumer", ownership="product", destination="bin/c-consumer"),
            declare("fixture", "build", "fixtures/frame.bin", FIXTURE, category="fixture", ownership="product", destination="fixtures/frame.bin"),
            self.bundled("opencv-core", "lib/libopencv_core.dylib", OPENCV_LIBRARY),
            declare("opencv-license", "opencv", "LICENSE", OPENCV_LICENSE_TEXT, category="notices", ownership="bundled", destination="notices/OPENCV-LICENSE", notice="opencv-license"),
            declare("model", "host", "models/det.onnx", MODEL, category="models", ownership="host"),
            declare("cudnn", "host", "cudnn64_8.dll", CUDNN, category="native_payload", ownership="host"),
        ]

    def stage(self, entries, *, destination=None, max_bytes=LIMIT):
        return staging.stage_files(entries, self.roots, destination or self.destination, max_bytes=max_bytes)

    def snapshot(self):
        return {
            (alias, path.relative_to(root).as_posix()): path.read_bytes() if path.is_file() else None
            for alias, root in self.roots.items()
            for path in root.rglob("*")
        }

    def assert_destination_untouched(self):
        self.assertEqual(os.listdir(self.destination), [])

    def test_stages_exact_bytes_and_accounts_by_ownership(self):
        entries = self.manifest()
        before = self.snapshot()

        result = self.stage(entries)

        self.assertEqual(self.snapshot(), before)
        for row in entries:
            if row["destination"] is None:
                continue
            staged = self.destination / row["destination"]
            self.assertEqual(staged.stat().st_size, row["bytes"])
            self.assertEqual(sha256(staged.read_bytes()), row["sha256"])
        self.assertFalse((self.destination / "models").exists())
        self.assertFalse((self.destination / "cudnn64_8.dll").exists())
        if os.name != "nt":
            self.assertTrue(os.access(self.destination / "bin" / "consumer", os.X_OK))
        self.assertEqual(result["files"], entries)
        for row in result["files"]:
            self.assertEqual(tuple(row), staging.ENTRY_KEYS)
            for value in row.values():
                if isinstance(value, str):
                    self.assertNotIn(str(self.base), value)
                    self.assertNotIn(str(self.base.resolve()), value)
        self.assertEqual(
            result["sizes"],
            {
                "rust_consumer": len(RUST_CONSUMER),
                "shared_library": len(CAPI_LIBRARY),
                "import_library": 0,
                "headers": 0,
                "native_payload": len(OPENCV_LIBRARY) + len(CUDNN),
                "notices": len(LICENSE_TEXT) + len(OPENCV_LICENSE_TEXT),
                "models": len(MODEL),
                "consumer": len(C_CONSUMER),
                "fixture": len(FIXTURE),
                "expanded_package": PACKAGE_BYTES,
                "host_supplied": len(MODEL) + len(CUDNN),
            },
        )

    def test_rejects_symlinks_in_final_and_intermediate_components(self):
        entries = self.manifest()
        library = self.roots["opencv"] / "lib" / "libopencv_core.dylib"
        try:
            os.symlink(library, library.with_name("libopencv_core.4.dylib"))
            os.symlink(self.roots["opencv"] / "lib", self.roots["opencv"] / "lib64", target_is_directory=True)
        except (OSError, NotImplementedError) as error:
            self.skipTest(f"symlinks unavailable: {error}")
        for path in ("lib/libopencv_core.4.dylib", "lib64/libopencv_core.dylib"):
            with self.subTest(path=path):
                with self.assertRaises(ValueError) as caught:
                    self.stage(entries + [self.bundled("alias", path, OPENCV_LIBRARY)])
                self.assertIn(f"opencv:{path}", str(caught.exception))
                self.assert_destination_untouched()

    @unittest.skipUnless(hasattr(os, "mkfifo"), "FIFOs unavailable")
    def test_rejects_special_files_without_opening_them(self):
        entries = self.manifest()
        os.mkfifo(self.roots["host"] / "models" / "pipe.onnx")
        entries.append(declare("pipe", "host", "models/pipe.onnx", b"", category="models", ownership="host"))
        with self.assertRaises(ValueError) as caught:
            self.stage(entries)
        self.assertIn("host:models/pipe.onnx", str(caught.exception))

    def test_rejects_windows_reparse_points(self):
        regular = SimpleNamespace(st_mode=stat.S_IFREG | 0o644, st_file_attributes=stat.FILE_ATTRIBUTE_ARCHIVE)
        placeholder = SimpleNamespace(
            st_mode=stat.S_IFREG | 0o644,
            st_file_attributes=stat.FILE_ATTRIBUTE_ARCHIVE | stat.FILE_ATTRIBUTE_REPARSE_POINT,
        )
        junction = SimpleNamespace(
            st_mode=stat.S_IFDIR | 0o755,
            st_file_attributes=stat.FILE_ATTRIBUTE_DIRECTORY | stat.FILE_ATTRIBUTE_REPARSE_POINT,
        )
        self.assertIsNone(staging._special_reason(regular, final=True))
        self.assertIsNotNone(staging._special_reason(placeholder, final=True))
        self.assertIsNotNone(staging._special_reason(junction, final=False))

    def test_rejects_inexact_directory_entry_names(self):
        entries = self.manifest()
        self.write("opencv", "lib/Extra.dylib", OPENCV_LIBRARY)
        entries.append(self.bundled("extra", "lib/extra.dylib", OPENCV_LIBRARY))
        case_insensitive = (self.roots["opencv"] / "lib" / "EXTRA.DYLIB").exists()
        with self.assertRaises(ValueError if case_insensitive else FileNotFoundError) as caught:
            self.stage(entries)
        self.assertIn("opencv:lib/extra.dylib", str(caught.exception))
        self.assert_destination_untouched()

    def test_rejects_unsafe_declared_paths_before_any_io(self):
        entries = self.manifest()
        unsafe = [
            "../escape",
            "/absolute",
            "lib//double",
            "./lib/x",
            "lib/./x",
            "lib\\x",
            "NUL.txt",
            "lib/com1",
            "lib/a:b",
            "lib/name.",
            "lib/ padded ",
            "lib/caf\u00e9",
            "lib/what?",
            "lib/x\n",
            "",
        ]
        for field in ("path", "destination"):
            for value in unsafe:
                with self.subTest(field=field, value=value):
                    bad = self.bundled("bad", "lib/libopencv_extra.dylib", OPENCV_LIBRARY)
                    bad[field] = value
                    with self.assertRaises(ValueError):
                        self.stage(entries + [bad])
                    self.assert_destination_untouched()

    def test_rejects_digest_and_length_mismatches(self):
        entries = self.manifest()
        self.write("opencv", "lib/libopencv_imgproc.dylib", OPENCV_LIBRARY)
        mismatched = {
            "digest": self.bundled("imgproc", "lib/libopencv_imgproc.dylib", OPENCV_LIBRARY, sha256=sha256(b"other")),
            "uppercase digest": self.bundled("imgproc", "lib/libopencv_imgproc.dylib", OPENCV_LIBRARY, sha256=sha256(OPENCV_LIBRARY).upper()),
            "declared shorter": self.bundled("imgproc", "lib/libopencv_imgproc.dylib", OPENCV_LIBRARY, bytes=len(OPENCV_LIBRARY) - 1),
            "declared longer": self.bundled("imgproc", "lib/libopencv_imgproc.dylib", OPENCV_LIBRARY, bytes=len(OPENCV_LIBRARY) + 1),
        }
        for index, (reason, entry) in enumerate(mismatched.items()):
            with self.subTest(reason=reason):
                destination = self.base / f"destination-{index}"
                destination.mkdir()
                with self.assertRaises(ValueError) as caught:
                    self.stage(entries + [entry], destination=destination)
                self.assertIn("imgproc", str(caught.exception))

    def test_streaming_reads_one_byte_past_the_declaration_then_refuses(self):
        declared = b"x" * 10
        path = self.write("host", "grown.bin", declared + b"y" * 5000)
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_BINARY", 0))
        try:
            with self.assertRaises(ValueError):
                staging._stream(fd, len(declared), sha256(declared), "host:grown.bin")
            self.assertEqual(os.lseek(fd, 0, os.SEEK_CUR), len(declared) + 1)
        finally:
            os.close(fd)

    def test_caps_aggregate_declared_bytes_including_host_data(self):
        entries = self.manifest()
        total = sum(row["bytes"] for row in entries)
        shipped = sum(row["bytes"] for row in entries if row["destination"] is not None)
        for cap in (total - 1, shipped):
            with self.subTest(cap=cap):
                with self.assertRaises(ValueError):
                    self.stage(entries, max_bytes=cap)
                self.assert_destination_untouched()
        self.stage(entries, max_bytes=total)

    def test_refuses_unresolved_bundling_models_and_cuda_closure(self):
        entries = self.manifest()
        self.write("opencv", "lib/libopencv_imgcodecs.dylib", OPENCV_LIBRARY)
        self.write("host", "models/rec.onnx", MODEL)
        self.write("host", "cudart64_12.dll", CUDNN)
        self.write("host", "cudnn_ops_infer64_8.dll", CUDNN)
        imgcodecs = "lib/libopencv_imgcodecs.dylib"
        refused = {
            "unresolved bundled": self.bundled("imgcodecs", imgcodecs, OPENCV_LIBRARY, redistribution="unresolved"),
            "shipped host-only": self.bundled("imgcodecs", imgcodecs, OPENCV_LIBRARY, redistribution="host-only"),
            "missing shipped notice": self.bundled("imgcodecs", imgcodecs, OPENCV_LIBRARY, notice="no-such-notice"),
            "notice naming a non-notices entry": self.bundled("imgcodecs", imgcodecs, OPENCV_LIBRARY, notice="capi"),
            "empty license": self.bundled("imgcodecs", imgcodecs, OPENCV_LIBRARY, license=""),
            "extra key": dict(self.bundled("imgcodecs", imgcodecs, OPENCV_LIBRARY), extra=1),
            "third-party bytes owned as product": declare("laundered", "opencv", imgcodecs, OPENCV_LIBRARY, category="native_payload", ownership="product", destination=imgcodecs),
            "bundled model": declare("rec", "host", "models/rec.onnx", MODEL, category="models", ownership="bundled", destination="models/rec.onnx", notice="opencv-license"),
            "host model approved": declare("rec", "host", "models/rec.onnx", MODEL, category="models", ownership="host", redistribution="approved"),
            "host model with destination": declare("rec", "host", "models/rec.onnx", MODEL, category="models", ownership="host", destination="models/rec.onnx"),
            "bundled cuDNN": declare("cudnn-ops", "host", "cudnn_ops_infer64_8.dll", CUDNN, category="native_payload", ownership="bundled", destination="lib/cudnn_ops_infer64_8.dll", notice="opencv-license"),
            "bundled CUDA runtime": declare("cudart", "host", "cudart64_12.dll", CUDNN, category="native_payload", ownership="bundled", destination="lib/cudart64_12.dll", notice="opencv-license"),
            "renamed CUDA runtime": declare("cudart", "host", "cudart64_12.dll", CUDNN, category="native_payload", ownership="bundled", destination="lib/runtime.dll", notice="opencv-license"),
            "CUDA-named destination": self.bundled("imgcodecs", imgcodecs, OPENCV_LIBRARY, destination="lib/cudnn64_8.dll"),
            "bundled CUDA provider": declare("cuda-ep", "host", "onnxruntime_providers_cuda.dll", CUDNN, category="native_payload", ownership="bundled", destination="lib/onnxruntime_providers_cuda.dll", notice="opencv-license"),
        }
        for reason, entry in refused.items():
            with self.subTest(reason=reason):
                with self.assertRaises(ValueError):
                    self.stage(entries + [entry])
                self.assert_destination_untouched()

        entries.append(declare("cudart-host", "host", "cudart64_12.dll", CUDNN, category="native_payload", ownership="host"))
        sizes = self.stage(entries)["sizes"]
        self.assertEqual(sizes["host_supplied"], len(MODEL) + 2 * len(CUDNN))
        self.assertEqual(sizes["native_payload"], len(OPENCV_LIBRARY) + 2 * len(CUDNN))
        self.assertEqual(sizes["expanded_package"], PACKAGE_BYTES)

    def test_rejects_duplicate_identities_and_destination_collisions(self):
        entries = self.manifest()
        self.write("opencv", "lib/libopencv_imgcodecs.dylib", OPENCV_LIBRARY)
        imgcodecs = "lib/libopencv_imgcodecs.dylib"
        duplicates = {
            "duplicate id": self.bundled("opencv-core", imgcodecs, OPENCV_LIBRARY),
            "case-insensitive destination": self.bundled("imgcodecs", imgcodecs, OPENCV_LIBRARY, destination="LIB/LIBOPENCV_CORE.DYLIB"),
            "destination inside a shipped file path": self.bundled("imgcodecs", imgcodecs, OPENCV_LIBRARY, destination="lib/libopencv_core.dylib/nested"),
            "duplicate host source": declare("model-again", "host", "models/det.onnx", MODEL, category="models", ownership="host"),
            "same source with another identity": self.bundled("core-again", "lib/libopencv_core.dylib", OPENCV_LIBRARY + b"x", destination="lib/libopencv_core.4.dylib"),
            "one source both shipped and host": declare("license-host", "opencv", "LICENSE", OPENCV_LICENSE_TEXT, category="native_payload", ownership="host"),
        }
        for reason, entry in duplicates.items():
            with self.subTest(reason=reason):
                with self.assertRaises(ValueError):
                    self.stage(entries + [entry])
                self.assert_destination_untouched()

    def test_deduplicates_category_bytes_but_counts_every_shipped_and_host_path(self):
        entries = self.manifest()
        self.write("opencv", "lib/libopencv_imgcodecs.dylib", OPENCV_LIBRARY)
        self.write("host", "models/rec.onnx", MODEL)
        entries += [
            dict(next(row for row in entries if row["id"] == "opencv-core"),
                 id="core-alias", destination="lib/libopencv_core.4.dylib"),
            self.bundled("imgcodecs", "lib/libopencv_imgcodecs.dylib", OPENCV_LIBRARY),
            declare("rec", "host", "models/rec.onnx", MODEL, category="models", ownership="host"),
        ]

        sizes = self.stage(entries)["sizes"]

        self.assertEqual((self.destination / "lib" / "libopencv_core.4.dylib").read_bytes(), OPENCV_LIBRARY)
        self.assertEqual((self.destination / "lib" / "libopencv_imgcodecs.dylib").read_bytes(), OPENCV_LIBRARY)
        self.assertEqual(sizes["native_payload"], 2 * len(OPENCV_LIBRARY) + len(CUDNN))
        self.assertEqual(sizes["expanded_package"], PACKAGE_BYTES + 2 * len(OPENCV_LIBRARY))
        self.assertEqual(sizes["models"], 2 * len(MODEL))
        self.assertEqual(sizes["host_supplied"], 2 * len(MODEL) + len(CUDNN))

    def test_same_source_cannot_change_its_license_at_another_destination(self):
        entries = self.manifest()
        original = next(row for row in entries if row["id"] == "opencv-core")
        entries.append(dict(original, id="relabelled", destination="lib/relabelled.dylib", license="different-license"))
        with self.assertRaises(ValueError):
            self.stage(entries)
        self.assert_destination_untouched()

    def test_source_root_link_is_not_hidden_by_canonicalization(self):
        entries = self.manifest()
        link = self.base / "root-link"
        try:
            os.symlink(self.roots["opencv"], link, target_is_directory=True)
        except (OSError, NotImplementedError) as error:
            self.skipTest(f"symlinks unavailable: {error}")
        self.roots["opencv"] = link
        with self.assertRaises(ValueError):
            self.stage(entries)
        self.assert_destination_untouched()

    def test_separates_package_bytes_from_host_supplied_bytes(self):
        bundled = self.stage(self.manifest())["sizes"]
        host_variant = [row for row in self.manifest() if row["id"] != "opencv-core"]
        host_variant.append(
            declare("opencv-core", "opencv", "lib/libopencv_core.dylib", OPENCV_LIBRARY, category="native_payload", ownership="host")
        )
        other = self.base / "destination-host"
        other.mkdir()

        host = self.stage(host_variant, destination=other)["sizes"]

        self.assertEqual(tuple(bundled), staging.SIZE_KEYS)
        self.assertEqual(tuple(host), staging.SIZE_KEYS)
        self.assertEqual(bundled["expanded_package"] - host["expanded_package"], len(OPENCV_LIBRARY))
        self.assertEqual(host["host_supplied"] - bundled["host_supplied"], len(OPENCV_LIBRARY))
        self.assertEqual(bundled["native_payload"], host["native_payload"])
        self.assertEqual(
            bundled["expanded_package"] + bundled["host_supplied"],
            host["expanded_package"] + host["host_supplied"],
        )
        self.assertFalse((other / "lib" / "libopencv_core.dylib").exists())

    def test_requires_exclusive_empty_destination_outside_roots(self):
        entries = self.manifest()
        (self.destination / "stale").write_bytes(b"")
        with self.assertRaises(ValueError):
            self.stage(entries)
        (self.destination / "stale").unlink()
        inside_root = self.roots["build"] / "out"
        inside_root.mkdir()
        with self.assertRaises(ValueError):
            self.stage(entries, destination=inside_root)
        with self.assertRaises(ValueError):
            self.stage([])
        self.assert_destination_untouched()

    def test_rejects_symlinked_destination(self):
        entries = self.manifest()
        link = self.base / "destination-link"
        try:
            os.symlink(self.destination, link, target_is_directory=True)
        except (OSError, NotImplementedError) as error:
            self.skipTest(f"symlinks unavailable: {error}")
        with self.assertRaises(ValueError):
            self.stage(entries, destination=link)
        self.assert_destination_untouched()

    def test_errors_name_relative_identities_not_host_paths(self):
        entries = self.manifest()
        entries.append(self.bundled("missing", "lib/libopencv_missing.dylib", OPENCV_LIBRARY))
        with self.assertRaises(FileNotFoundError) as caught:
            self.stage(entries)
        message = str(caught.exception)
        self.assertIn("opencv:lib/libopencv_missing.dylib", message)
        self.assertNotIn(str(self.base), message)
        self.assertNotIn(str(self.base.resolve()), message)
        self.assert_destination_untouched()

    def test_archive_is_deterministic_and_covers_only_package_rows(self):
        entries = self.manifest()
        other = self.base / "destination-again"
        other.mkdir()
        results = [(self.stage(entries), self.destination), (self.stage(entries, destination=other), other)]
        archives = []
        for index, (result, destination) in enumerate(results):
            archive = self.base / f"package-{index}.zip"
            report = staging.write_package_archive(result["files"], destination, archive)
            self.assertEqual(report["bytes"], archive.stat().st_size)
            self.assertEqual(report["sha256"], sha256(archive.read_bytes()))
            archives.append((archive.read_bytes(), report))
        self.assertEqual(archives[0], archives[1])

        expected = [
            "LICENSE",
            "bin/consumer",
            "lib/libmado_pilot_capi.dylib",
            "lib/libopencv_core.dylib",
            "notices/OPENCV-LICENSE",
        ]
        self.assertEqual(archives[0][1]["entries"], len(expected))
        with zipfile.ZipFile(self.base / "package-0.zip") as bundle:
            self.assertEqual(bundle.namelist(), expected)
            self.assertEqual(bundle.read("lib/libopencv_core.dylib"), OPENCV_LIBRARY)
            self.assertEqual(bundle.read("bin/consumer"), RUST_CONSUMER)
            self.assertEqual({info.date_time for info in bundle.infolist()}, {staging.ARCHIVE_DATE_TIME})

    def test_archive_reverifies_staged_files_and_stays_outside_destination(self):
        result = self.stage(self.manifest())
        archive = self.base / "package.zip"
        with self.assertRaises(ValueError):
            staging.write_package_archive(result["files"], self.destination, self.destination / "package.zip")
        self.assertFalse((self.destination / "package.zip").exists())

        (self.destination / "lib" / "libopencv_core.dylib").write_bytes(b"opencv" * 49 + b"OPENCV")
        with self.assertRaises(ValueError) as caught:
            staging.write_package_archive(result["files"], self.destination, archive)
        self.assertIn("destination:lib/libopencv_core.dylib", str(caught.exception))
        self.assertNotIn(str(self.base), str(caught.exception))

        with self.assertRaises(FileExistsError):
            staging.write_package_archive(result["files"], self.destination, archive)


if __name__ == "__main__":
    unittest.main()
