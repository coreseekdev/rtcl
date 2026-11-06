#!/usr/bin/env python3
"""比较 tclsh / rtcl 的正则语义语料输出并归类差异。

用法: python3 specs/regex/analyze.py tclsh.out rtcl.out
"""
import collections
import re
import sys

LINE = re.compile(r"PAT<(.*)> STR<(.*)> => (\d+) <(.*)>")
LAZY = re.compile(r"[?*+}]\?")
ALT = re.compile(r"\|")


def parse(path):
    out = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            m = LINE.match(line.rstrip("\n"))
            if m:
                out[(m.group(1), m.group(2))] = (m.group(3), m.group(4))
    return out


def main():
    oracle = parse(sys.argv[1])
    rtcl = parse(sys.argv[2])
    diffs = [(k, oracle[k], rtcl.get(k)) for k in oracle if oracle[k] != rtcl.get(k)]
    cats = collections.Counter()
    for (pat, _), _, _ in diffs:
        lazy = bool(LAZY.search(pat))
        alt = bool(ALT.search(pat))
        if lazy and alt:
            cats["lazy+alternation"] += 1
        elif lazy:
            cats["lazy (SHORTER mode)"] += 1
        elif alt:
            cats["leftmost-longest vs first"] += 1
        else:
            cats["POSIX submatch/other"] += 1
    total = len(oracle)
    print(f"cases={total} diff={len(diffs)} ({100.0 * len(diffs) / max(total, 1):.1f}%)")
    for name, n in cats.most_common():
        print(f"  {name}: {n}")
    for (pat, s), ov, rv in diffs[:10]:
        print(f"  e.g. {pat!r} @ {s!r}: tclsh={ov} rtcl={rv}")


if __name__ == "__main__":
    main()
