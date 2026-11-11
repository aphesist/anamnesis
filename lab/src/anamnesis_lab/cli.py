"""``anamnesis-lab`` command line.

    anamnesis-lab demo transcendentalist [--small] [--out DIR]
    anamnesis-lab demo pragmatist [--small] [--out DIR]
    anamnesis-lab demo curate --from out/transcendentalist --out ../demo/transcendentalist [--target-notes N]
    anamnesis-lab bench retrieval --dataset scifact --models hash,e5-small
    anamnesis-lab bench links --bundle out/transcendentalist --models hash
    anamnesis-lab bench layout --bundle out/transcendentalist [--rust-placements FILE]
    anamnesis-lab report [--results results/]

Every network access happens behind one of these explicit commands; the tests never
download anything.
"""

from __future__ import annotations

import argparse
import json
import logging
import sys
from pathlib import Path

LAB_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CACHE = LAB_ROOT / "cache"
DEFAULT_OUT = LAB_ROOT / "out"
DEFAULT_RESULTS = LAB_ROOT / "results"


def _demo(args: argparse.Namespace) -> int:
    if args.theme == "curate":
        return _curate(args)
    from .demo import build_theme

    out = Path(args.out) if args.out else DEFAULT_OUT / (args.theme + ("-small" if args.small else ""))
    manifest = build_theme(args.theme, out, Path(args.cache), small=args.small)
    c = manifest["counts"]
    print(f"wrote {c['notes']} notes to {out / 'notes'}")
    print(json.dumps({"counts": c, "date_range": manifest["date_range"]}, indent=2, ensure_ascii=False))
    return 0


def _curate(args: argparse.Namespace) -> int:
    from .demo.curate import curate

    src = Path(args.src) if args.src else DEFAULT_OUT / "transcendentalist"
    out = Path(args.out) if args.out else LAB_ROOT.parent / "demo" / "transcendentalist"
    m = curate(src, out, target_notes=args.target_notes, max_bytes=args.max_bytes, min_chars=args.min_chars)
    c = m["counts"]
    print(f"curated {c['notes']} of {m['curation']['from_notes']} notes ({c['bytes'] / 1e6:.2f} MB) into {out / 'notes'}")
    print(json.dumps({k: v for k, v in c.items() if k != "years_with_journal"}, indent=2, ensure_ascii=False))
    return 0


def _bench(args: argparse.Namespace) -> int:
    results = Path(args.results)
    if args.what == "retrieval":
        from .bench.retrieval import run_retrieval

        run_retrieval(
            datasets=args.dataset.split(","), models=args.models.split(","), cache=Path(args.cache),
            results=results, max_docs=args.max_docs, max_queries=args.max_queries,
        )
    elif args.what == "links":
        from .bench.linkpred import run_linkpred

        run_linkpred(Path(args.bundle), models=args.models.split(","), cache=Path(args.cache), results=results,
                     split=args.split, targets=args.targets)
    elif args.what == "layout":
        from .bench.layout import run_layout

        run_layout(Path(args.bundle), model=args.models.split(",")[0], cache=Path(args.cache), results=results,
                   rust_placements=Path(args.rust_placements) if args.rust_placements else None,
                   snapshots=args.snapshots, max_items=args.max_items, methods=args.methods.split(","))
    return 0


def _report(args: argparse.Namespace) -> int:
    from .bench.report import render_markdown

    print(render_markdown(Path(args.results)))
    return 0


def main(argv: list[str] | None = None) -> int:
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    p = argparse.ArgumentParser(prog="anamnesis-lab", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)

    d = sub.add_parser("demo", help="build a public-domain demo brain")
    d.add_argument("theme", choices=["transcendentalist", "pragmatist", "curate"],
                   help="a theme to build, or `curate` to pick the compact demo shipped in demo/ from a full build")
    d.add_argument("--small", action="store_true", help="offline build from the bundled fixture (~300 entries)")
    d.add_argument("--out", help="output folder (default: lab/out/<theme>; curate: demo/transcendentalist)")
    d.add_argument("--from", dest="src", help="curate: a full build (default: lab/out/transcendentalist)")
    d.add_argument("--target-notes", type=int, default=1100, help="curate: at most this many notes")
    d.add_argument("--max-bytes", type=int, default=3_900_000, help="curate: at most this much markdown")
    d.add_argument("--min-chars", type=int, default=400, help="curate: shortest journal entry used to fill the timeline")
    d.add_argument("--cache", default=str(DEFAULT_CACHE), help="download cache (default: lab/cache)")
    d.set_defaults(func=_demo)

    b = sub.add_parser("bench", help="run a benchmark (downloads data and models)")
    b.add_argument("what", choices=["retrieval", "links", "layout"])
    b.add_argument("--dataset", default="scifact", help="retrieval: comma list of scifact,nfcorpus")
    b.add_argument("--models", default="hash", help="comma list: hash, bm25, e5-small, minilm")
    b.add_argument("--bundle", help="links/layout: a demo bundle folder (with manifest.json)")
    b.add_argument("--split", type=float, default=0.5, help="links: queries are notes after this time quantile")
    b.add_argument("--targets", default="content", choices=["content", "all"],
                   help="links: score links to entries/essays only, or also to person/index notes")
    b.add_argument("--rust-placements", help="layout: placements json exported from the rust layout")
    b.add_argument("--snapshots", type=int, default=5, help="layout: number of incremental snapshots")
    b.add_argument("--max-items", type=int, default=4000, help="layout: subsample the bundle to this many notes")
    b.add_argument("--methods", default="umap,pacmap", help="layout: comma list of umap, pacmap")
    b.add_argument("--max-docs", type=int, default=None, help="retrieval: cap corpus size (for quick runs)")
    b.add_argument("--max-queries", type=int, default=None, help="retrieval: cap number of queries")
    b.add_argument("--cache", default=str(DEFAULT_CACHE))
    b.add_argument("--results", default=str(DEFAULT_RESULTS))
    b.set_defaults(func=_bench)

    r = sub.add_parser("report", help="print results/*.json as markdown tables")
    r.add_argument("--results", default=str(DEFAULT_RESULTS))
    r.set_defaults(func=_report)

    args = p.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
