#!/usr/bin/env python3
"""Spencer Rust 引擎（M3）捕获验收：随机 4000 例 vs tclsh -indices -inline。

用法：specs/tclmem.sh 'python3 specs/regex/spencer-captures.py'
（脚本用已构建的 target/release/examples 二进制；缺失时 cargo run）。
"""
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
os.chdir(ROOT)
PAIRS = re.compile(r"^t \{(.*?)\} \{(.*?)\}$")
GROUP = re.compile(r"\{(-?\d+) (-?\d+)\}")


def parse_pairs():
    out = []
    with open("specs/regex/random-4000.tcl", encoding="utf-8") as f:
        for line in f:
            m = PAIRS.match(line.rstrip("\n"))
            if m:
                out.append((m.group(1), m.group(2)))
    return out


def main() -> int:
    pairs = parse_pairs()
    with open("/tmp/spencer_cap_pairs.tsv", "w", encoding="utf-8") as f:
        for pat, text in pairs:
            f.write(pat + "\t" + text.encode("utf-8").hex() + "\n")
    with open("/tmp/spencer_cap_oracle.tcl", "w", encoding="utf-8") as f:
        for pat, text in pairs:
            f.write("set rc [catch {regexp -indices -inline -- {%s} {%s}} m]\n" % (pat, text))
            f.write("if {$rc} { puts ERR } else { puts $m }\n")
    subprocess.run(["tclsh", "/tmp/spencer_cap_oracle.tcl"], check=True,
                   stdout=open("/tmp/spencer_cap_oracle.out", "w"))
    bin_path = None
    for cand in ("target/release/examples/spencer_captures",
                 "target/debug/examples/spencer_captures"):
        if os.path.exists(cand):
            bin_path = cand
            break
    if bin_path is None:
        subprocess.run(
            ["cargo", "run", "-q", "-p", "rtcl-vm", "--features", "regexp-spencer",
             "--example", "spencer_captures"],
            check=True, stdin=open("/tmp/spencer_cap_pairs.tsv"),
            stdout=open("/tmp/spencer_cap_ours.out", "w"))
    else:
        subprocess.run([bin_path], check=True,
                       stdin=open("/tmp/spencer_cap_pairs.tsv"),
                       stdout=open("/tmp/spencer_cap_ours.out", "w"))
    oracle = [l.rstrip("\n") for l in open("/tmp/spencer_cap_oracle.out", encoding="utf-8")]
    ours = [l.rstrip("\n") for l in open("/tmp/spencer_cap_ours.out", encoding="utf-8")]

    def norm_oracle(line: str) -> str:
        if not line:
            return "NONE"
        groups = GROUP.findall(line)
        if not groups:
            return "ERR:" + line
        # tclsh -indices 已是闭区间；空匹配 end=start-1。
        return " ".join(f"{s} {e}" for s, e in groups)

    diffs = []
    for i, (pat, text) in enumerate(pairs):
        a = norm_oracle(oracle[i])
        b = ours[i] or "NONE"
        if a != b:
            diffs.append((i, pat, text, a, b))
    print(f"spencer-captures: cases={len(pairs)} diff={len(diffs)}")
    for i, pat, text, a, b in diffs[:20]:
        print(f"  {pat!r} @ {text!r}\n    tclsh={a}\n    spencer={b}")
    return 0 if not diffs else 1


if __name__ == "__main__":
    sys.exit(main())
