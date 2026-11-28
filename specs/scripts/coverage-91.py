#!/usr/bin/env python3
"""Coverage matrix: which tclsh 9.1 commands / ensemble subcommands have
behavioural rules in specs/*.allium.

A command counts as covered when it appears as a word inside a `when:`
script (comments and prose do not count). This is a *reachability* check,
not a proof of completeness: it tells you which commands no rule ever
exercises.

Usage:
    python3 specs/scripts/coverage-91.py
    python3 specs/scripts/coverage-91.py --min 2      # show commands < 2 hits
    TCLSH_91=/path/to/tclsh9.1 python3 specs/scripts/coverage-91.py
"""
import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SPECS = REPO / "specs"
TCL = os.environ.get("TCLSH_91", str(REPO / ".refer/tcl910/inst/bin/tclsh9.1"))

# Ensembles whose subcommand surface is worth tracking. Extend as specs grow.
ENSEMBLES = [
    "after", "array", "binary", "chan", "clock", "dict", "encoding", "file",
    "info", "interp", "namespace", "package", "string", "tcl::prefix",
    "timer", "unicode", "zipfs",
]

MENU_RE = re.compile(
    r"must be (?:one of )?(.*?)\.?$", re.S
)


def oracle(script: str) -> str:
    r = subprocess.run([TCL], input=script, text=True, capture_output=True, timeout=20)
    return r.stdout


def all_commands() -> list[str]:
    out = oracle("puts [join [lsort [info commands]] \\n]")
    return [c for c in out.split() if c]


def subcommands(ens: str) -> list[str]:
    out = oracle(
        "catch {%s __nope_xyz__} m; puts $m" % ens
    )
    m = MENU_RE.search(out.strip())
    if not m:
        return []
    body = m.group(1)
    body = body.replace("or ", "")
    parts = [p.strip().strip("\"") for p in body.split(",")]
    return [p for p in parts if p and " " not in p]


def when_scripts() -> list[str]:
    scripts = []
    for p in SPECS.glob("*.allium"):
        for line in p.read_text().splitlines():
            s = line.strip()
            if s.startswith("when:") and "Evaluates(" in s:
                scripts.append(s)
    return scripts


def hits(scripts: list[str], word: str, sub: str | None = None) -> int:
    if sub:
        pat = re.compile(
            r"(?<![\w:])" + re.escape(word) + r"\s+" + re.escape(sub) + r"(?![\w-])"
        )
    else:
        pat = re.compile(r"(?<![\w:])" + re.escape(word) + r"(?![\w:])")
    return sum(1 for s in scripts if pat.search(s))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--min", type=int, default=1, help="show commands with fewer hits")
    ap.add_argument("--subs", action="store_true", help="also report per-ensemble subcommands")
    args = ap.parse_args()

    if not Path(TCL).exists():
        print(f"oracle not found: {TCL}", file=sys.stderr)
        return 2

    scripts = when_scripts()
    cmds = all_commands()
    gaps = [(c, hits(scripts, c)) for c in cmds]
    gaps = [(c, n) for c, n in gaps if n < args.min]
    print(f"commands={len(cmds)} when-scripts={len(scripts)} below-min={len(gaps)}")
    for c, n in gaps:
        print(f"  {n:3d} {c}")
    if args.subs:
        for ens in ENSEMBLES:
            subs = subcommands(ens)
            if not subs:
                continue
            missing = [(s, hits(scripts, ens, s)) for s in subs]
            missing = [(s, n) for s, n in missing if n < args.min]
            if missing:
                print(f"\n{ens}: {len(missing)}/{len(subs)} subcommands below min")
                for s, n in missing:
                    print(f"  {n:3d} {ens} {s}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
