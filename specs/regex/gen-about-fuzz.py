#!/usr/bin/env python3
"""生成 regexp -about 的随机 re_info 标志差分语料（扩展 atom 集）。

用法:
  python3 specs/regex/gen-about-fuzz.py [seed] [count] > /tmp/aboutfuzz.tcl
  tclsh /tmp/aboutfuzz.tcl > o.txt
  ./target/release/rtcl -f /tmp/aboutfuzz.tcl > r.txt
  diff o.txt r.txt

已知残差（2026-11-07）：扩展 atom 集 10000 例中 10 例涉及
\\y/\\m/\\M 与回引/可选节点的联立求解（仅 -about 标志，匹配语义
无差异）；标准 atom 集 3000 例为 0。
"""
import random, sys

seed = int(sys.argv[1]) if len(sys.argv) > 1 else 20261111
count = int(sys.argv[2]) if len(sys.argv) > 2 else 10000
random.seed(seed)
atoms = ["a","b","(a)","(a*)","(a|b)","[ab]","[[:alpha:]]",r"\d",r"\w",r"\s",r"\y",r"\m",r"\M",r"\Y",r"\A",r"\Z","^","$",".","(?:a)","(?:)","()","(?=a)","(?!a)","(a)\\1","a*","a+","a?","a{2}","a{0,2}","a{,2}","a{b}","{","}","|","a*?","a+?","a{1,2}?","[^a]","[a-c]","[a-]","[-a]","a|","|a"]
pats=set()
while len(pats)<count:
    n=random.randint(1,5)
    p="".join(random.choice(atoms) for _ in range(n))
    pref=random.choice(["","(?b)","(?e)","***:","***="])
    p=pref+p
    if len(p)<=60: pats.add(p)
print("proc hx {s} {binary decode hex $s}")
print("proc t {ph} {")
print("  set pat [hx $ph]")
print("  set rc [catch {regexp -about -- $pat} m]")
print('  puts "P<$ph> => $rc <$m>"')
print("}")
for p in sorted(pats): print("t {%s}" % p.encode().hex())
