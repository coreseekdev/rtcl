#!/usr/bin/env python3
"""生成 BRE/ERE 方言随机差分语料（确定性 seed）。

用法: python3 specs/regex/gen-dialect-corpus.py > /tmp/dialect-fuzz.tcl
然后分别用 tclsh 与 RTCL_REGEX_ENGINE=spencer rtcl 运行并 diff。
pattern/text 以 UTF-8 十六进制传递，避免 Tcl 花括号/反斜杠转义干扰。
"""
import random

random.seed(20261106)
alpha = list("ab()[]{}*+?|.^$\\12,-dn<>") + [" ", "0"]

pats = set()
while len(pats) < 2500:
    n = random.randint(1, 10)
    body = "".join(random.choice(alpha) for _ in range(n))
    pref = random.choice(["(?b)", "(?e)", "(?b)", "(?e)", ""])
    p = pref + body
    if len(p) <= 24:
        pats.add(p)

strs = ["", "a", "b", "aa", "ab", "aaa", "aab", "abab", "abb", "ba",
        "a|b", "(a)", "a+b", "{2}", "dn", "a b", "\\", "1", "2", "0", "*"]

print("proc t {ph sh} {")
print("  set pat [binary decode hex $ph]")
print("  set str [binary decode hex $sh]")
print("  set rc [catch {regexp -inline -- $pat $str} m]")
print('  puts "P<$ph> S<$sh> => $rc <$m>"')
print("}")
for p in sorted(pats):
    ph = p.encode().hex()
    for s in strs:
        print("t {%s} {%s}" % (ph, s.encode().hex()))
