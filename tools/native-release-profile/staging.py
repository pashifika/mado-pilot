"""Stage declared candidate files into an empty caller-owned directory with exact identity checks.

Frozen interface: ``stage_files(entries, roots, destination, *, max_bytes)`` returns
``{"files": [...], "sizes": {...}}``. Every declared file is located below its root
without following links, streamed once through SHA-256 (shipped files also into the
destination) and must match its declared length and digest. Failures raise
``ValueError``/``OSError`` with relative labels only; the caller discards its scratch.
"""

from __future__ import annotations

import hashlib
import os
import re
import stat
import zipfile
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from pathlib import Path, PurePosixPath

ENTRY_KEYS = (
    "id",
    "root",
    "path",
    "destination",
    "category",
    "ownership",
    "sha256",
    "bytes",
    "source",
    "version",
    "license",
    "notice",
    "redistribution",
)
SOURCE_METADATA = ("category", "ownership", "sha256", "bytes", "source", "version", "license", "notice", "redistribution")
# Package bytes; ``consumer``/``fixture`` are staged qualification inputs, not package content.
PACKAGE_CATEGORIES = frozenset(
    {"rust_consumer", "shared_library", "import_library", "headers", "native_payload", "notices"}
)
CATEGORIES = PACKAGE_CATEGORIES | {"models", "consumer", "fixture"}
OWNERSHIP_BY_CATEGORY = {
    "rust_consumer": frozenset({"product"}),
    "shared_library": frozenset({"product"}),
    "import_library": frozenset({"product"}),
    "headers": frozenset({"product"}),
    "native_payload": frozenset({"bundled", "host"}),
    "notices": frozenset({"bundled", "product"}),
    "models": frozenset({"host"}),
    "consumer": frozenset({"product"}),
    "fixture": frozenset({"product"}),
}
REDISTRIBUTIONS = frozenset({"approved", "host-only", "unresolved"})
SIZE_KEYS = (
    "rust_consumer",
    "shared_library",
    "import_library",
    "headers",
    "native_payload",
    "notices",
    "models",
    "consumer",
    "fixture",
    "expanded_package",
    "host_supplied",
)
# Lower-cased library stems (leading "lib" removed) of the CUDA/cuDNN closure; never bundled.
CUDA_FAMILY_STEMS = (
    "cuda",
    "cudart",
    "cudnn",
    "cublas",
    "cublaslt",
    "cufft",
    "cufftw",
    "curand",
    "cusparse",
    "cusparselt",
    "cusolver",
    "cupti",
    "nvblas",
    "nvrtc",
    "nvjitlink",
    "nvinfer",
    "nvonnxparser",
    "nvcuda",
    "nccl",
    "onnxruntime_providers_cuda",
    "onnxruntime_providers_tensorrt",
)
CHUNK_BYTES = 1 << 20
ARCHIVE_DATE_TIME = (1980, 1, 1, 0, 0, 0)

_SHA256 = re.compile(r"[0-9a-f]{64}")
_ALIAS = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,63}")
_PATH_PART_FORBIDDEN = frozenset('\\:*?"<>|')
_WINDOWS_RESERVED = frozenset(
    {"CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"}
    | {f"COM{digit}" for digit in range(10)}
    | {f"LPT{digit}" for digit in range(10)}
)
_READ_FLAGS = (
    os.O_RDONLY
    | getattr(os, "O_NOFOLLOW", 0)
    | getattr(os, "O_BINARY", 0)
    | getattr(os, "O_CLOEXEC", 0)
)
_CREATE_FLAGS = (
    os.O_WRONLY
    | os.O_CREAT
    | os.O_EXCL
    | getattr(os, "O_NOFOLLOW", 0)
    | getattr(os, "O_BINARY", 0)
    | getattr(os, "O_CLOEXEC", 0)
)


def stage_files(
    entries: list[dict],
    roots: dict[str, Path],
    destination: Path,
    *,
    max_bytes: int,
) -> dict:
    """Verify every declared file, copy the shipped ones into ``destination``, report inventory and sizes."""
    if isinstance(max_bytes, bool) or not isinstance(max_bytes, int) or max_bytes <= 0:
        raise ValueError("max_bytes must be a positive int")
    resolved_roots = _resolve_roots(roots)
    rows = _validate_entries(entries, set(resolved_roots))
    declared_total = sum(row["bytes"] for row in rows)
    if declared_total > max_bytes:
        raise ValueError(
            f"declared bytes {declared_total} exceed max_bytes {max_bytes} (host data included)"
        )
    destination_root = _resolve_destination(destination, resolved_roots)

    # Inspect every source before the first write so shape mismatches leave the destination empty.
    listing = _Listing()
    sources: list[tuple[Path, os.stat_result]] = []
    canonical_rows: dict[Path, dict] = {}
    for row in rows:
        label = _source_label(row)
        source, source_stat = _locate(resolved_roots[row["root"]], PurePosixPath(row["path"]), label, listing)
        if source_stat.st_size != row["bytes"]:
            raise ValueError(f"{label}: source has {source_stat.st_size} bytes, declared {row['bytes']}")
        previous = canonical_rows.get(source)
        if previous is not None and (previous["destination"] is None or row["destination"] is None or
                                     any(previous[key] != row[key] for key in SOURCE_METADATA)):
            raise ValueError(f"{label}: inconsistent canonical source declaration")
        canonical_rows[source] = row
        sources.append((source, source_stat))

    for row, (source, source_stat) in zip(rows, sources):
        label = _source_label(row)
        if row["destination"] is None:
            with _open_regular(source, source_stat, label) as source_fd:
                _stream(source_fd, row["bytes"], row["sha256"], label)
            continue
        target = _prepare_destination(destination_root, PurePosixPath(row["destination"]))
        target_label = f"destination:{row['destination']}"
        with _open_regular(source, source_stat, label) as source_fd:
            with _scoped_errors(target_label):
                target_fd = os.open(target, _CREATE_FLAGS, 0o600)
            try:
                _stream(source_fd, row["bytes"], row["sha256"], label, _fd_writer(target_fd, target_label))
                with _scoped_errors(target_label):
                    if os.fstat(target_fd).st_size != row["bytes"]:
                        raise OSError(None, "short write", target_label)
            finally:
                os.close(target_fd)
        with _scoped_errors(target_label):
            os.chmod(target, stat.S_IMODE(source_stat.st_mode) & 0o777)

    return {"files": rows, "sizes": _sizes(rows, [source for source, _ in sources])}


def snapshot_executable(source: Path, destination: Path | None, sha256: str, *, max_bytes: int) -> int:
    """Copy external bytes, or reverify an already-private staged file without relocating it."""
    directory = _resolve_directory(source.parent, "executable-root")
    found, metadata = _locate(directory, PurePosixPath(source.name), "executable", _Listing())
    if metadata.st_size > max_bytes:
        raise ValueError("executable snapshot exceeds remaining staging bytes")
    if destination is None and metadata.st_nlink != 1:
        raise ValueError("staged executable must have no external hard-link alias")
    with _open_regular(found, metadata, "executable") as source_fd:
        if destination is None:
            _stream(source_fd, metadata.st_size, sha256, "executable")
        else:
            descriptor = os.open(destination, _CREATE_FLAGS, 0o600)
            try:
                _stream(source_fd, metadata.st_size, sha256, "executable", _fd_writer(descriptor, "executable-snapshot"))
            finally:
                os.close(descriptor)
    if os.name != "nt":
        os.chmod(source if destination is None else destination, 0o500)
    return 0 if destination is None else metadata.st_size


def write_package_archive(files: list[dict], destination: Path, archive: Path) -> dict:
    """Write the shipped ``PACKAGE_CATEGORIES`` rows of a ``stage_files`` inventory into a deterministic ZIP.

    Rows are sorted by destination and re-verified against their inventory length and
    digest while read. Fixed 1980-01-01 timestamps, Unix attributes from the staged mode
    and zlib's default deflate level make the bytes reproducible for one Python/zlib
    build; cross-host identity is not claimed. ``archive`` must not exist and must lie
    outside ``destination``. Returns ``format``, ``compression``, ``entries``, ``bytes``, ``sha256``.
    """
    rows = []
    for index, row in enumerate(files):
        if not isinstance(row, dict) or "destination" not in row or "category" not in row:
            raise ValueError(f"row {index}: must be an inventory row")
        if row["destination"] is not None and row["category"] in PACKAGE_CATEGORIES:
            rows.append(_archive_row(row))
    if not rows:
        raise ValueError("no shipped package rows to archive")
    rows.sort(key=lambda row: row["destination"])
    _reject_destination_collisions(rows)
    destination_root = _resolve_directory(destination, "destination")
    archive_path = Path(archive)
    if not archive_path.is_absolute():
        raise ValueError("archive must be an absolute path")
    with _scoped_errors("archive"):
        archive_parent = archive_path.parent.resolve(strict=True)
    if archive_parent == destination_root or archive_parent.is_relative_to(destination_root):
        raise ValueError("archive must lie outside destination")

    listing = _Listing()
    with _scoped_errors("archive"):
        archive_fd = os.open(archive_path, _CREATE_FLAGS, 0o644)
    with os.fdopen(archive_fd, "wb") as stream, zipfile.ZipFile(stream, "w", zipfile.ZIP_DEFLATED) as bundle:
        for row in rows:
            label = f"destination:{row['destination']}"
            staged, staged_stat = _locate(destination_root, PurePosixPath(row["destination"]), label, listing)
            info = zipfile.ZipInfo(row["destination"], date_time=ARCHIVE_DATE_TIME)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.create_system = 3
            info.external_attr = (stat.S_IFREG | (stat.S_IMODE(staged_stat.st_mode) & 0o777)) << 16
            info.file_size = row["bytes"]
            with _open_regular(staged, staged_stat, label) as staged_fd:
                with _scoped_errors("archive"):
                    member = bundle.open(info, "w", force_zip64=row["bytes"] > zipfile.ZIP64_LIMIT)
                with member:
                    _stream(staged_fd, row["bytes"], row["sha256"], label, _labelled_writer(member.write, "archive"))

    digest = hashlib.sha256()
    total = 0
    with _scoped_errors("archive"), open(archive_path, "rb") as stream:
        for chunk in iter(lambda: stream.read(CHUNK_BYTES), b""):
            total += len(chunk)
            digest.update(chunk)
    return {
        "format": "zip",
        "compression": "deflate",
        "entries": len(rows),
        "bytes": total,
        "sha256": digest.hexdigest(),
    }


# --- declaration validation ---------------------------------------------------


def _validate_entries(entries: object, aliases: set[str]) -> list[dict]:
    if not isinstance(entries, (list, tuple)) or not entries:
        raise ValueError("entries must be a non-empty list")
    rows: list[dict] = []
    ids: set[str] = set()
    sources: dict[tuple[str, str], dict] = {}
    for index, entry in enumerate(entries):
        row = _validate_entry(entry, index, aliases)
        if row["id"] in ids:
            raise ValueError(f"entry {row['id']!r}: duplicate id")
        ids.add(row["id"])
        # One source may be shipped under several destinations; any other repetition is a duplicate.
        source_key = (row["root"], row["path"])
        previous = sources.get(source_key)
        if previous is None:
            sources[source_key] = row
        elif previous["destination"] is None or row["destination"] is None:
            raise ValueError(f"entry {row['id']!r}: duplicate source {_source_label(row)!r}")
        elif any(previous[key] != row[key] for key in SOURCE_METADATA):
            raise ValueError(f"entry {row['id']!r}: source {_source_label(row)!r} declared with inconsistent metadata")
        rows.append(row)

    shipped = [row for row in rows if row["destination"] is not None]
    _reject_destination_collisions(shipped)
    # Every redistributed file must ship its notice: ``notice`` names a shipped ``notices`` entry.
    notices = {row["id"] for row in shipped if row["category"] == "notices"}
    for row in shipped:
        if row["notice"] not in notices:
            raise ValueError(f"entry {row['id']!r}: notice {row['notice']!r} is not the id of a shipped notices entry")
    return rows


def _validate_entry(entry: object, index: int, aliases: set[str]) -> dict:
    if not isinstance(entry, dict):
        raise ValueError(f"entry {index}: must be an object")
    if set(entry) != set(ENTRY_KEYS):
        raise ValueError(
            f"entry {index}: keys drifted: missing={sorted(set(ENTRY_KEYS) - set(entry))!r}, "
            f"extra={sorted(set(entry) - set(ENTRY_KEYS))!r}"
        )
    identifier = _require_text(entry["id"], f"entry {index}: id")
    label = f"entry {identifier!r}"
    root = entry["root"]
    if not isinstance(root, str) or root not in aliases:
        raise ValueError(f"{label}: root must be one of the supplied root aliases")
    path = str(_require_relative_path(entry["path"], f"{label}: path"))
    category = entry["category"]
    if category not in CATEGORIES:
        raise ValueError(f"{label}: category {category!r} is not one of {sorted(CATEGORIES)}")
    ownership = entry["ownership"]
    if ownership not in OWNERSHIP_BY_CATEGORY[category]:
        raise ValueError(
            f"{label}: ownership {ownership!r} is not allowed for category {category!r} "
            f"(allowed: {sorted(OWNERSHIP_BY_CATEGORY[category])})"
        )
    redistribution = entry["redistribution"]
    if redistribution not in REDISTRIBUTIONS:
        raise ValueError(f"{label}: redistribution {redistribution!r} is not one of {sorted(REDISTRIBUTIONS)}")
    if redistribution == "unresolved":
        raise ValueError(f"{label}: redistribution is unresolved; resolve the disposition before staging")
    destination = entry["destination"]
    if ownership == "host":
        if destination is not None:
            raise ValueError(f"{label}: host-supplied files are verified in place; destination must be null")
        if redistribution != "host-only":
            raise ValueError(f"{label}: host-supplied files must declare redistribution 'host-only'")
    else:
        if destination is None:
            raise ValueError(f"{label}: {ownership} files are shipped; destination is required")
        if redistribution != "approved":
            raise ValueError(f"{label}: shipped files must declare redistribution 'approved'")
        destination = str(_require_relative_path(destination, f"{label}: destination"))
        if ownership == "bundled":
            for name in (PurePosixPath(path).name, PurePosixPath(destination).name):
                if _cuda_family(name):
                    raise ValueError(f"{label}: {name!r} is a CUDA/cuDNN library and stays host-provided")
    sha256 = entry["sha256"]
    if not isinstance(sha256, str) or _SHA256.fullmatch(sha256) is None:
        raise ValueError(f"{label}: sha256 must be 64 lowercase hex digits")
    size = entry["bytes"]
    if isinstance(size, bool) or not isinstance(size, int) or size < 0:
        raise ValueError(f"{label}: bytes must be a non-negative int")
    for field in ("source", "version", "license"):
        _require_text(entry[field], f"{label}: {field}")
    notice = entry["notice"]
    if not isinstance(notice, str) or (destination is not None and not notice):
        raise ValueError(f"{label}: notice must be a string naming a shipped notices entry")
    return {
        "id": identifier,
        "root": root,
        "path": path,
        "destination": destination,
        "category": category,
        "ownership": ownership,
        "sha256": sha256,
        "bytes": size,
        "source": entry["source"],
        "version": entry["version"],
        "license": entry["license"],
        "notice": notice,
        "redistribution": redistribution,
    }


def _require_text(value: object, field: str) -> str:
    if not isinstance(value, str) or not value.strip() or not value.isprintable():
        raise ValueError(f"{field} must be a non-empty printable string")
    return value


def _require_relative_path(value: object, field: str) -> PurePosixPath:
    """Canonical printable-ASCII POSIX relative path that is also safe on Windows."""
    if not isinstance(value, str) or not value:
        raise ValueError(f"{field} must be a non-empty POSIX relative path")
    if not value.isascii() or not value.isprintable():
        raise ValueError(f"{field} must be printable ASCII: {value!r}")
    for part in value.split("/"):
        if part in ("", ".", ".."):
            raise ValueError(f"{field} is not a canonical relative path: {value!r}")
        if _PATH_PART_FORBIDDEN.intersection(part):
            raise ValueError(f"{field} contains a character unsafe on Windows: {value!r}")
        if part != part.strip(" ") or part.endswith("."):
            raise ValueError(f"{field} has a component with leading/trailing space or trailing dot: {value!r}")
        if part.split(".", 1)[0].upper() in _WINDOWS_RESERVED:
            raise ValueError(f"{field} uses a Windows reserved device name: {value!r}")
    return PurePosixPath(value)


def _cuda_family(name: str) -> bool:
    stem = name.lower()
    if stem.startswith("lib"):
        stem = stem[3:]
    return any(
        stem == family or (stem.startswith(family) and not stem[len(family)].isalpha())
        for family in CUDA_FAMILY_STEMS
    )


def _reject_destination_collisions(rows: list[dict]) -> None:
    """Destinations collide when equal after case folding or when one is inside a shipped file path."""
    files: dict[str, str] = {}
    for row in rows:
        key = row["destination"].casefold()
        if key in files:
            raise ValueError(f"entry {row['id']!r}: destination {row['destination']!r} collides with entry {files[key]!r}")
        files[key] = row["id"]
    for row in rows:
        parent = PurePosixPath(row["destination"]).parent
        while parent.name:
            if str(parent).casefold() in files:
                raise ValueError(f"entry {row['id']!r}: destination {row['destination']!r} is inside a shipped file path")
            parent = parent.parent


def _archive_row(row: dict) -> dict:
    identifier = _require_text(row.get("id"), "row id")
    label = f"row {identifier!r}"
    destination = str(_require_relative_path(row["destination"], f"{label}: destination"))
    sha256 = row.get("sha256")
    if not isinstance(sha256, str) or _SHA256.fullmatch(sha256) is None:
        raise ValueError(f"{label}: sha256 must be 64 lowercase hex digits")
    size = row.get("bytes")
    if isinstance(size, bool) or not isinstance(size, int) or size < 0:
        raise ValueError(f"{label}: bytes must be a non-negative int")
    return {"id": identifier, "destination": destination, "sha256": sha256, "bytes": size}


def _source_label(row: dict) -> str:
    return f"{row['root']}:{row['path']}"


# --- filesystem -----------------------------------------------------------------


class _Listing:
    """Exact directory-entry names per directory, read once per call."""

    def __init__(self) -> None:
        self._names: dict[Path, frozenset[str]] = {}

    def contains(self, directory: Path, name: str) -> bool:
        names = self._names.get(directory)
        if names is None:
            names = frozenset(os.listdir(directory))
            self._names[directory] = names
        return name in names


@contextmanager
def _scoped_errors(label: str) -> Iterator[None]:
    """Re-raise OSError naming the relative label so host paths never reach evidence."""
    try:
        yield
    except OSError as error:
        raise OSError(error.errno, error.strerror or type(error).__name__, label) from None


def _special_reason(entry_stat: os.stat_result, *, final: bool) -> str | None:
    if getattr(entry_stat, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
        return "is a Windows reparse point"
    if stat.S_ISLNK(entry_stat.st_mode):
        return "is a symbolic link"
    if final:
        return None if stat.S_ISREG(entry_stat.st_mode) else "is not a regular file"
    return None if stat.S_ISDIR(entry_stat.st_mode) else "is not a directory"


def _locate(base: Path, relative: PurePosixPath, label: str, listing: _Listing) -> tuple[Path, os.stat_result]:
    """Walk ``relative`` below ``base`` with lstat and exact entry names; return the regular file and its stat."""
    current = base
    last = len(relative.parts) - 1
    for index, part in enumerate(relative.parts):
        candidate = current / part
        with _scoped_errors(label):
            entry_stat = os.lstat(candidate)
            exact = listing.contains(current, part)
        if not exact:
            raise ValueError(f"{label}: {part!r} is not the exact directory entry name")
        reason = _special_reason(entry_stat, final=index == last)
        if reason is not None:
            raise ValueError(f"{label}: {part!r} {reason}")
        if index == last:
            return candidate, entry_stat
        current = candidate
    raise ValueError(f"{label}: empty path")


@contextmanager
def _open_regular(path: Path, expected: os.stat_result, label: str) -> Iterator[int]:
    """Open read-only and prove the opened file is the inspected regular file."""
    with _scoped_errors(label):
        fd = os.open(path, _READ_FLAGS)
    try:
        opened = os.fstat(fd)
        same_inode = not (opened.st_ino and expected.st_ino) or (
            (opened.st_dev, opened.st_ino) == (expected.st_dev, expected.st_ino)
        )
        if _special_reason(opened, final=True) is not None or not same_inode:
            raise ValueError(f"{label}: file changed between inspection and open")
        yield fd
    finally:
        os.close(fd)


def _fd_writer(fd: int, label: str) -> Callable[[bytes], None]:
    def write(chunk: bytes) -> None:
        view = memoryview(chunk)
        with _scoped_errors(label):
            while view:
                view = view[os.write(fd, view) :]

    return write


def _labelled_writer(write: Callable[[bytes], object], label: str) -> Callable[[bytes], None]:
    def labelled(chunk: bytes) -> None:
        with _scoped_errors(label):
            write(chunk)

    return labelled


def _stream(
    fd: int,
    expected_bytes: int,
    expected_sha256: str,
    label: str,
    write: Callable[[bytes], None] | None = None,
) -> None:
    """Hash at most ``expected_bytes + 1`` bytes (copying them through ``write``); require the declared identity."""
    digest = hashlib.sha256()
    total = 0
    remaining = expected_bytes + 1
    while remaining:
        with _scoped_errors(label):
            chunk = os.read(fd, min(CHUNK_BYTES, remaining))
        if not chunk:
            break
        total += len(chunk)
        if total > expected_bytes:
            raise ValueError(f"{label}: source is longer than the declared {expected_bytes} bytes")
        remaining -= len(chunk)
        digest.update(chunk)
        if write is not None:
            write(chunk)
    if total != expected_bytes:
        raise ValueError(f"{label}: source has {total} bytes, declared {expected_bytes}")
    actual = digest.hexdigest()
    if actual != expected_sha256:
        raise ValueError(f"{label}: SHA-256 {actual} does not match declared {expected_sha256}")


def _resolve_directory(path: object, label: str) -> Path:
    given = Path(path)
    if not given.is_absolute():
        raise ValueError(f"{label} must be an absolute path")
    with _scoped_errors(label):
        given_stat = os.lstat(given)
    reason = _special_reason(given_stat, final=False)
    if reason is not None:
        raise ValueError(f"{label} {reason}")
    with _scoped_errors(label):
        return given.resolve(strict=True)


def _resolve_roots(roots: object) -> dict[str, Path]:
    if not isinstance(roots, dict):
        raise ValueError("roots must be a dict of alias to absolute directory")
    resolved: dict[str, Path] = {}
    seen: dict[Path, str] = {}
    for alias, root in roots.items():
        if not isinstance(alias, str) or _ALIAS.fullmatch(alias) is None:
            raise ValueError(f"root alias {alias!r} is not a safe identifier")
        if not isinstance(root, (str, os.PathLike)):
            raise ValueError(f"root {alias!r} must be a path")
        given = Path(root)
        if not given.is_absolute():
            raise ValueError(f"root {alias!r} must be an absolute path")
        label = f"root:{alias}"
        directory = _resolve_directory(given, label)
        if directory in seen:
            raise ValueError(f"root {alias!r} resolves to the same directory as {seen[directory]!r}")
        seen[directory] = alias
        resolved[alias] = directory
    return resolved


def _resolve_destination(destination: object, roots: dict[str, Path]) -> Path:
    resolved = _resolve_directory(destination, "destination")
    with _scoped_errors("destination"):
        occupied = bool(os.listdir(resolved))
    if occupied:
        raise ValueError("destination must be an empty directory")
    for alias, root in roots.items():
        if resolved == root or resolved.is_relative_to(root) or root.is_relative_to(resolved):
            raise ValueError(f"destination overlaps root {alias!r}")
    return resolved


def _prepare_destination(destination_root: Path, relative: PurePosixPath) -> Path:
    """Create the parent directories of ``relative`` inside the owned destination, refusing links."""
    current = destination_root
    for part in relative.parts[:-1]:
        current = current / part
        label = f"destination:{current.relative_to(destination_root).as_posix()}"
        with _scoped_errors(label):
            try:
                current_stat = os.lstat(current)
            except FileNotFoundError:
                os.mkdir(current, 0o755)
                continue
        reason = _special_reason(current_stat, final=False)
        if reason is not None:
            raise ValueError(f"{label} {reason}")
    return current / relative.parts[-1]


def _sizes(rows: list[dict], sources: list[Path]) -> dict[str, int]:
    """Deduplicate canonical source paths per category; package and host totals count paths."""
    sizes = {key: 0 for key in SIZE_KEYS}
    counted: dict[str, set[Path]] = {category: set() for category in CATEGORIES}
    for row, source in zip(rows, sources):
        category = row["category"]
        if source not in counted[category]:
            counted[category].add(source)
            sizes[category] += row["bytes"]
        if row["destination"] is None:
            sizes["host_supplied"] += row["bytes"]
        elif category in PACKAGE_CATEGORIES:
            sizes["expanded_package"] += row["bytes"]
    return sizes
