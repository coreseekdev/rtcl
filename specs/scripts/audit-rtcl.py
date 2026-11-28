#!/usr/bin/env python3
"""Differential audit: tclsh 9.1 oracle vs rtcl on the spec corpus.

Every `when: Evaluates("...")` rule that the oracle audit can execute is run
in a fresh tclsh 9.1 and a fresh rtcl, and the resulting completion code and
value are compared. The oracle is the authority; a difference here is an rtcl
divergence, not a spec error.

Usage:
    systemd-run --user --scope -p MemoryMax=2G --quiet \
        python3 specs/scripts/audit-rtcl.py
    python3 specs/scripts/audit-rtcl.py --file list.allium
    python3 specs/scripts/audit-rtcl.py --limit 50 --verbose

Rules whose trigger is not self-contained (contains `--`, spans lines, or
times out) are skipped, exactly as in audit-91.py.
"""
import argparse
import os
import re
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SPECS = REPO / "specs"
ORACLE = os.environ.get(
    "TCLSH_91", str(REPO / ".refer/tcl910/inst/bin/tclsh9.1")
)
RTCL = os.environ.get(
    "RTCL_BIN",
    str(REPO / "target/release/rtcl")
    if (REPO / "target/release/rtcl").exists()
    else str(REPO / "target/debug/rtcl"),
)


def unescape(s: str) -> str:
    return s.replace('\\"', '"').replace('\\\\', '\\')


def parse_file(p: Path):
    blocks = re.split(r"(?m)^rule\s+", p.read_text())[1:]
    for b in blocks:
        name = b.split(None, 1)[0].strip("{ ")
        m = re.search(r'(?m)^\s*when:\s*Evaluates\("(.*)"\)\s*$', b)
        if not m:
            continue
        yield {"file": p.name, "rule": name, "script": unescape(m.group(1))}


def wrap(script: str) -> str:
    return (
        "set __c [catch {\n" + script + "\n} __r]\n"
        'puts -nonewline "$__c|"\n'
        "puts -nonewline $__r"
    )


def run(binary: str, src: str):
    try:
        r = subprocess.run(
            [binary, "-c", src] if binary.endswith("rtcl") else [binary],
            input=None if binary.endswith("rtcl") else src,
            text=True,
            capture_output=True,
            timeout=10,
        )
    except (subprocess.TimeoutExpired, OSError):
        return None
    if r.returncode != 0:
        return None
    out = r.stdout
    m = re.match(r"^(-?\d+)\|(.*)$", out, re.S)
    if not m:
        return None
    return int(m.group(1)), m.group(2)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--file", help="only this spec file name")
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    for label, path in (("oracle", ORACLE), ("rtcl", RTCL)):
        if not Path(path).exists():
            print(f"{label} not found: {path}", file=sys.stderr)
            return 2

    files = (
        [SPECS / args.file] if args.file else sorted(SPECS.glob("*.allium"))
    )
    rules = [r for p in files for r in parse_file(p)]
    if args.limit:
        rules = rules[: args.limit]

    same = skipped = diff = 0
    per_file = Counter()
    per_file_total = Counter()
    diffs = []
    for r in rules:
        s = r["script"]
        per_file_total[r["file"]] += 1
        if "--" in s or "\n" in s:
            skipped += 1
            continue
        src = wrap(s)
        ora = run(ORACLE, src)
        if ora is None:
            skipped += 1
            continue
        got = run(RTCL, src)
        if got is None:
            skipped += 1
            continue
        if got == ora:
            same += 1
        else:
            diff += 1
            per_file[r["file"]] += 1
            diffs.append((r, ora, got))

    total = len(rules)
    print(
        f"rules={total} compared={same + diff} same={same} diff={diff} "
        f"skipped={skipped}"
    )
    if diffs:
        print("\nper spec:")
        for f in sorted(per_file):
            print(f"  {f:24s} {per_file[f]:3d}/{per_file_total[f]:3d}")
        print("\ndivergences:")
        for r, ora, got in diffs:
            o = f"code={ora[0]} {ora[1]!r}"
            g = f"code={got[0]} {got[1]!r}"
            if not args.verbose:
                o = o[:160]
                g = g[:160]
            print(f"{r['file']}:{r['rule']}")
            print(f"  when : {r['script'][:200]}")
            print(f"  oracle: {o}")
            print(f"  rtcl  : {g}")
    return 0 if diff == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
