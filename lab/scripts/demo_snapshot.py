"""Regenerate the prebuilt demo brain in demo/transcendentalist/snapshot/.

The snapshot is generated data: the curated transcendentalist demo (public-domain texts, see
demo/transcendentalist/SOURCES.md) imported by a real `anamnesis demo`, embedded with the
default local model (multilingual-e5-small), clustered, laid out and given link suggestions.
The server seeds `<data dir>/anamnesis/demo/` from it, so the demo opens at once with no model
download.

What this script does, in order:

1. `anamnesis demo --rebuild --data-dir <work> --demo-bundle demo/transcendentalist`
   (needs the local model: ~465 MB downloaded once into the usual model cache);
2. `anamnesis verify` on the built brain;
3. packs the brain directory: `VACUUM INTO` a fresh rollback-journal database (no wal, no free
   pages), then gzip -9 each of brain.sqlite, mind_state.json, views.json and jobs.json with a
   fixed header (no name, mtime 0), and writes manifest.json (sha256 and sizes of each file,
   event count and head hash, item and chunk counts, the embedding model). The mcp token and
   anything else in the brain directory are left out.

The procedure is reproducible; the bytes are not, because events carry the time they were
recorded and the brain draws its own hold-out seed. Python standard library only.

    cargo build --release -p anamnesis-server
    python3 lab/scripts/demo_snapshot.py                  # build, verify, pack
    python3 lab/scripts/demo_snapshot.py --from-brain DIR # pack an already built demo brain
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
NOTES = REPO / "demo" / "transcendentalist"
OUT = NOTES / "snapshot"
FILES = ["brain.sqlite", "mind_state.json", "views.json", "jobs.json"]
FORMAT = 1


def default_bin() -> Path:
    exe = "anamnesis.exe" if os.name == "nt" else "anamnesis"
    target = Path(os.environ.get("CARGO_TARGET_DIR", REPO / "target"))
    return target / "release" / exe


def run(cmd: list[str]) -> str:
    print("$", " ".join(cmd), file=sys.stderr, flush=True)
    r = subprocess.run(cmd, check=True, stdout=subprocess.PIPE, text=True)
    return r.stdout


def build(binary: Path, work: Path) -> Path:
    run([str(binary), "demo", "--rebuild", "--data-dir", str(work), "--demo-bundle", str(NOTES)])
    brain = work / "anamnesis" / "demo"
    print(run([str(binary), "verify", "--brain-dir", str(brain)]).strip(), file=sys.stderr)
    return brain


def vacuum_into(src: Path, dest: Path) -> None:
    if dest.exists():
        dest.unlink()
    con = sqlite3.connect(src)
    try:
        con.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        con.execute("VACUUM INTO ?", (str(dest),))
    finally:
        con.close()
    con = sqlite3.connect(dest)
    try:
        con.execute("PRAGMA journal_mode=DELETE")
    finally:
        con.close()


def facts(db: Path) -> dict:
    con = sqlite3.connect(db)
    try:
        q = lambda sql: con.execute(sql).fetchall()  # noqa: E731
        events = q("SELECT COUNT(*) FROM events")[0][0]
        head = q("SELECT hash FROM events ORDER BY seq DESC LIMIT 1")[0][0]
        items = q("SELECT COUNT(*) FROM items")[0][0]
        chunks = q("SELECT COUNT(*) FROM chunks WHERE text != ''")[0][0]
        models = [m for (m,) in q("SELECT DISTINCT model_id FROM embeddings ORDER BY model_id")]
    finally:
        con.close()
    if len(models) != 1:
        sys.exit(f"expected vectors of one model in the demo brain, found {models}")
    if models[0].startswith("hash@"):
        sys.exit("the demo brain was embedded with the hash embedder; build it with the local model")
    return {"events": events, "head": head, "items": items, "chunks": chunks, "embedding_model": models[0]}


def gz(src: Path, dest: Path) -> None:
    with open(src, "rb") as fin, open(dest, "wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", compresslevel=9, fileobj=raw, mtime=0) as fout:
            shutil.copyfileobj(fin, fout, 1 << 20)


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def pack(brain: Path, out: Path, version: str) -> dict:
    out.mkdir(parents=True, exist_ok=True)
    for old in out.glob("*.gz"):
        old.unlink()
    with tempfile.TemporaryDirectory() as td:
        stage = Path(td)
        vacuum_into(brain / "brain.sqlite", stage / "brain.sqlite")
        for name in FILES[1:]:
            if (brain / name).is_file():
                shutil.copyfile(brain / name, stage / name)
        manifest = {"format": FORMAT, **facts(stage / "brain.sqlite"), "files": {}}
        for name in FILES:
            p = stage / name
            if not p.is_file():
                continue
            gz(p, out / f"{name}.gz")
            manifest["files"][name] = {
                "sha256": sha256(p),
                "bytes": p.stat().st_size,
                "gz_bytes": (out / f"{name}.gz").stat().st_size,
            }
    manifest["anamnesis_version"] = version
    manifest["note"] = (
        "generated data: the curated transcendentalist demo (public-domain texts, see ../SOURCES.md) "
        f"built by `anamnesis demo` with {manifest['embedding_model']}. "
        "regenerate with: python3 lab/scripts/demo_snapshot.py"
    )
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    return manifest


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--bin", type=Path, default=default_bin(), help="the anamnesis binary (default: the release build)")
    ap.add_argument("--from-brain", type=Path, help="pack this built demo brain directory instead of building one")
    ap.add_argument("--work", type=Path, help="data dir for the build (default: a temporary one)")
    ap.add_argument("--out", type=Path, default=OUT)
    a = ap.parse_args()
    if not a.bin.is_file():
        sys.exit(f"{a.bin} not found: cargo build --release -p anamnesis-server, or pass --bin")
    version = run([str(a.bin), "--version"]).strip()
    if a.from_brain:
        m = pack(a.from_brain, a.out, version)
    else:
        with tempfile.TemporaryDirectory() as td:
            work = a.work or Path(td)
            m = pack(build(a.bin, work), a.out, version)
    total = sum(f["gz_bytes"] for f in m["files"].values())
    raw = sum(f["bytes"] for f in m["files"].values())
    print(
        f"wrote {a.out}: {m['items']} items, {m['events']} events, {m['chunks']} chunks, "
        f"{raw / 1e6:.1f} MB -> {total / 1e6:.1f} MB gzip",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
