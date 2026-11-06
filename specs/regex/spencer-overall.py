#!/usr/bin/env python3
"""Spencer Rust 引擎（M2）整体匹配验收：随机 4000 例 vs tclsh。

用法（建议经 512M 护栏）：
  specs/tclmem.sh 'python3 specs/regex/spencer-overall.py'

前置：cargo build -p rtcl-vm --features regexp-spencer --example spencer_span
（脚本内部用 cargo run -q 构建/运行）。
"""
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
os.chdir(ROOT)
PAIRS = re.compile(r"^t \{(.*?)\} \{(.*?)\}$")


def parse_pairs() -> list[tuple[str, str]]:
    out = []
    with open("specs/regex/random-4000.tcl", encoding="utf-8") as f:
        for line in f:
            m = PAIRS.match(line.rstrip("\n"))
            if m:
                out.append((m.group(1), m.group(2)))
    return out


def main() -> int:
    pairs = parse_pairs()
    with open("/tmp/spencer_pairs.tsv", "w", encoding="utf-8") as f:
        for pat, text in pairs:
            f.write(pat + "\t" + text.encode("utf-8").hex() + "\n")
    with open("/tmp/spencer_oracle.tcl", "w", encoding="utf-8") as f:
        for pat, text in pairs:
            f.write("set rc [catch {regexp -indices -inline -- {%s} {%s}} m]\n" % (pat, text))
            f.write("if {$rc} { puts ERR } else { puts $m }\n")
    subprocess.run(["tclsh", "/tmp/spencer_oracle.tcl"], check=True,
                   stdout=open("/tmp/spencer_oracle.out", "w"))
    subprocess.run(
        ["cargo", "run", "-q", "-p", "rtcl-vm", "--features", "regexp-spencer",
         "--example", "spencer_span"],
        check=True, stdin=open("/tmp/spencer_pairs.tsv"),
        stdout=open("/tmp/spencer_ours.out", "w"),
    )
    oracle = [l.rstrip("\n") for l in open("/tmp/spencer_oracle.out", encoding="utf-8")]
    ours = [l.rstrip("\n") for l in open("/tmp/spencer_ours.out", encoding="utf-8")]

    def norm(line: str) -> str:
        if not line:
            return "NONE"
        m = re.match(r"\{(-?\d+) (-?\d+)\}", line)
        if not m:
            return "ERR:" + line
        s, e = int(m.group(1)), int(m.group(2))
        if s < 0:
            return "NONE"
        end = s if e == s - 1 else e + 1  # Tcl 索引是闭区间
        return f"{s} {end}"

    diffs = [(i, pairs[i], norm(oracle[i]), ours[i] or "NONE")
             for i in range(len(pairs)) if norm(oracle[i]) != (ours[i] or "NONE")]
    print(f"spencer-overall: cases={len(pairs)} diff={len(diffs)}")
    for i, (pat, text), ov, rv in diffs[:20]:
        print(f"  {pat!r} @ {text!r}: tclsh={ov} spencer={rv}")
    return 0 if not diffs else 1


if __name__ == "__main__":
    sys.exit(main())
