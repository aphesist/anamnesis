#!/usr/bin/env python3
"""Pack the demo brain's anatomy view into site/hero/brain-data.bin.

Stdlib only. Reads the shipped demo snapshot (views.json.gz for positions,
brain.sqlite.gz for region membership and labels) and writes a small binary
the landing-page hero fetches.

Usage (from the repository root):
    python3 site/hero/build/extract.py [snapshot_dir] [out_path]

Layout (little-endian):
    0   4  magic "ABRN"
    4   1  version (1)
    5   1  number of regions R (point region index < R, 255 = none)
    6   2  number of points N
    8   4  float32 scale: xyz = int16 / 32767 * scale
    12  6N int16 x, y, z per point (anatomy frame: x lateral, y up, z front)
    ..  N  uint8 region index, regions ordered by size (largest first)
    ..  2  uint16 byte length L of the label JSON
    ..  L  utf-8 JSON: [{"t": label, "a": anatomy, "n": size, "c": [x, y, z]}, ...]
           for the largest regions (at most MAX_LABELS), c = member centroid
"""
import gzip
import json
import os
import sqlite3
import struct
import sys
import tempfile

MAX_LABELS = 12

root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
snap = sys.argv[1] if len(sys.argv) > 1 else os.path.join(root, "demo", "transcendentalist", "snapshot")
out = sys.argv[2] if len(sys.argv) > 2 else os.path.join(root, "site", "hero", "brain-data.bin")

with gzip.open(os.path.join(snap, "views.json.gz"), "rt", encoding="utf-8") as f:
    views = json.load(f)
ids, pos = views["ids"], views["anatomy"]
n = len(ids)
assert n == len(pos) and 0 < n < 65536

with tempfile.TemporaryDirectory() as tmp:
    db_path = os.path.join(tmp, "brain.sqlite")
    with gzip.open(os.path.join(snap, "brain.sqlite.gz"), "rb") as src, open(db_path, "wb") as dst:
        while True:
            chunk = src.read(1 << 20)
            if not chunk:
                break
            dst.write(chunk)
    db = sqlite3.connect(db_path)
    (run_regions,) = db.execute("SELECT regions FROM region_runs ORDER BY seq DESC LIMIT 1").fetchone()
    member = dict(db.execute("SELECT item_id, region_id FROM placements"))
    db.close()

regions = [r for r in json.loads(run_regions) if not r.get("died_at")]
regions.sort(key=lambda r: -int(r.get("size", 0)))
regions = regions[:254]
index = {r["id"]: k for k, r in enumerate(regions)}
reg = [index.get(member.get(i), 255) for i in ids]

scale = max(abs(c) for p in pos for c in p) or 1.0
q = [max(-32767, min(32767, round(c / scale * 32767))) for p in pos for c in p]

labels = []
for k, r in enumerate(regions[:MAX_LABELS]):
    pts = [pos[i] for i in range(n) if reg[i] == k]
    if not pts:
        continue
    c = [round(sum(p[d] for p in pts) / len(pts), 4) for d in range(3)]
    labels.append({"t": r["label"].lower(), "a": (r.get("anatomy") or "").lower(), "n": len(pts), "c": c})
meta = json.dumps(labels, ensure_ascii=False, separators=(",", ":")).encode("utf-8")

buf = bytearray(b"ABRN")
buf += struct.pack("<BBHf", 1, len(regions), n, scale)
buf += struct.pack("<%dh" % len(q), *q)
buf += bytes(reg)
buf += struct.pack("<H", len(meta)) + meta
os.makedirs(os.path.dirname(out), exist_ok=True)
with open(out, "wb") as f:
    f.write(buf)
print("wrote %s: %d points, %d regions, %d labels, %d bytes" % (out, n, len(regions), len(labels), len(buf)))
