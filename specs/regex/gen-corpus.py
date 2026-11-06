#!/usr/bin/env python3
"""生成随机正则语义差分语料（确定性 seed）。

用法: python3 specs/regex/gen-corpus.py > specs/regex/random-4000.tcl
"""
import random

random.seed(20261106)
atoms = [
    "a", "b", "ab", "[ab]", "(a)", "(b)", "(a|b)", "(a|ab)", "(ab|a)",
    "(a?)", "(a*)", "(a+)",
]
quants = ["", "*", "+", "?", "{1,2}", "*?", "+?", "??", "{1,2}?"]

pats = set()
while len(pats) < 400:
    n = random.randint(1, 3)
    parts = [random.choice(atoms) + random.choice(quants) for _ in range(n)]
    pat = "".join(parts)
    if random.random() < 0.25:
        pat = "(" + pat + ")"
    if random.random() < 0.15:
        pat += "$"
    if len(pat) <= 28:
        pats.add(pat)

strs = ["", "a", "b", "aa", "ab", "aaa", "aab", "abab", "abb", "ba"]
print("proc t {pat str} {")
print(" set rc [catch {regexp -inline -- $pat $str} m]")
print(' puts "PAT<$pat> STR<$str> => $rc <$m>"')
print("}")
for p in sorted(pats):
    for s in strs:
        print("t {%s} {%s}" % (p, s))
