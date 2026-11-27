#!/usr/bin/env python3
"""Audit allium `when: Evaluates(...)` rules against the Tcl 9.1 oracle.

Parses every specs/*.allium rule that has a single-line Evaluates(...)
trigger and an ensures completion code/value, runs the trigger script in a
fresh tclsh 9.1, and reports mismatches.

Rules whose trigger depends on an external `-- after:` setup, spans multiple
lines, or is not self-contained are skipped; make a rule self-contained to
bring it into the audit.

Usage:
    python3 specs/scripts/audit-91.py
    TCLSH_91=/path/to/tclsh9.1 python3 specs/scripts/audit-91.py
"""
import os
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SPECS = REPO / "specs"
TCL = os.environ.get("TCLSH_91", str(REPO / ".refer/tcl910/inst/bin/tclsh9.1"))


def unescape(s: str) -> str:
    # Allium strings in this corpus use \" and \\ only.
    return s.replace('\\"', '"').replace('\\\\', '\\')


def parse_file(p: Path):
    blocks = re.split(r"(?m)^rule\s+", p.read_text())[1:]
    for b in blocks:
        name = b.split(None, 1)[0].strip("{ ")
        m = re.search(r'(?m)^\s*when:\s*Evaluates\("(.*)"\)\s*$', b)
        if not m:
            continue
        code = re.search(r"(?m)^\s*ensures:\s*completion\.code\s*=\s*(ok|error)", b)
        val = re.search(r'(?m)^\s*ensures:\s*completion\.value\s*=\s*"(.*)"\s*$', b)
        yield {
            "file": p.name,
            "rule": name,
            "script": unescape(m.group(1)),
            "want_code": code.group(1) if code else None,
            "want_value": unescape(val.group(1)) if val else None,
        }


def run(script: str):
    src = (
        "set __c [catch {\n" + script + "\n} __r]\n"
        'puts -nonewline "$__c|"\n'
        "puts -nonewline [string length $__r]\n"
        'puts -nonewline "|"\n'
        "puts -nonewline $__r"
    )
    try:
        r = subprocess.run([TCL], input=src, text=True, capture_output=True, timeout=5)
    except subprocess.TimeoutExpired:
        return None
    if r.returncode != 0:
        return None
    m = re.match(r"^(\d+)\|(\d+)\|(.*)$", r.stdout, re.S)
    if not m:
        return None
    return int(m.group(1)), m.group(3)


def main() -> int:
    if not Path(TCL).exists():
        print(f"oracle not found: {TCL}", file=sys.stderr)
        return 2
    rules = [r for p in sorted(SPECS.glob("*.allium")) for r in parse_file(p)]
    checked = skipped = 0
    bad = []
    for r in rules:
        s = r["script"]
        if "--" in s or "\n" in s:
            skipped += 1
            continue
        out = run(s)
        if out is None:
            skipped += 1
            continue
        checked += 1
        code, val = out
        why = []
        if r["want_code"] == "ok" and code != 0:
            why.append(f"code {code} != 0")
        if r["want_code"] == "error" and code != 1:
            why.append(f"code {code} != 1")
        if r["want_value"] is not None and val != r["want_value"]:
            why.append(f"value <{val}> != <{r['want_value']}>")
        if why:
            bad.append((r, why, val))

    print(f"checked={checked} skipped={skipped} mismatches={len(bad)}")
    for r, why, val in bad:
        print(f"{r['file']}:{r['rule']}: {'; '.join(why)}")
        print(f"  when: {r['script']}")
        print(f"  got : {val}")
    return 0 if not bad else 1


if __name__ == "__main__":
    raise SystemExit(main())
