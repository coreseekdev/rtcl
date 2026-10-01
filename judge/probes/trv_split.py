#!/usr/bin/env python3
"""Split a gen corpus file into per-case scripts and diff tclsh vs rtcl.

Usage: python3 trv_split.py <corpus.tcl> [case-id ...]
Prints one block per differing case:
  == <case-id> == DIFF
  oracle rc=<n>: <stdout+stderr>
  rtcl   rc=<n>: <stdout+stderr>
"""
import os, re, subprocess, sys

CASE_RE = re.compile(r"^# case: (\S+) cmd=(.*)$", re.M)
RTCL = os.environ.get("RTCL", "/tmp/rtcl_trace")

def main():
    path = sys.argv[1]
    only = set(sys.argv[2:])
    with open(path) as f:
        text = f.read()
    marks = [(m.group(1), m.start()) for m in CASE_RE.finditer(text)]
    marks.append(("__END__", len(text)))
    for i in range(len(marks) - 1):
        cid, start = marks[i]
        end = marks[i + 1][1]
        if only and cid not in only:
            continue
        p = "/tmp/trv_case_%s.tcl" % cid.replace(".", "_")
        with open(p, "w") as f:
            f.write(text[start:end])
        o = subprocess.run(["tclsh", p], capture_output=True, timeout=20)
        r = subprocess.run([RTCL, "-f", p], capture_output=True, timeout=20)
        oout = (o.stdout + o.stderr).decode("utf-8", "replace")
        rout = (r.stdout + r.stderr).decode("utf-8", "replace")
        same = oout == rout and o.returncode == r.returncode
        if not same:
            print("== %s == DIFF   (%s)" % (cid, p))
            print("  oracle rc=%d: %r" % (o.returncode, oout))
            print("  rtcl   rc=%d: %r" % (r.returncode, rout))

main()
