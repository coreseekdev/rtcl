#!/usr/bin/env python3
"""生成 regsub 替换串展开随机差分语料（确定性 seed）。

覆盖 `&`、`\\0`-`\\9`、`\\&`、`\\\\`、未匹配组、`\\10`、尾随反斜杠、
`-all/-start/-nocase` 与 var/无 var 形式。

用法:
  python3 specs/regex/gen-regsub-fuzz.py [seed] [count] > /tmp/regsubfuzz.tcl
  tclsh /tmp/regsubfuzz.tcl > o.txt
  ./target/release/rtcl -f /tmp/regsubfuzz.tcl > r.txt
  diff o.txt r.txt
"""
import random, sys
seed = int(sys.argv[1]) if len(sys.argv) > 1 else 20261120
count = int(sys.argv[2]) if len(sys.argv) > 2 else 4000
random.seed(seed)
pats=["(a)(b)?","(a)|(b)","()","(a*)","()(a)","(a)(b)(c)","(?:(a))","(a)?(b)?","(a)(b)(c)(d)"]
texts=["","a","b","ab","abc","aa","ba","abcd"]
specs=["","X","&","&&",r"\0",r"\1",r"\2",r"\3",r"\9",r"\10",r"\11",r"\&",r"\\",r"\\1",r"\x",r"a\b",r"[\1]",r"\0\0","pre&post",r"\\&",r"\\\1",r"\1-\2",r"&-\1","\\"]
flagsets=[[],["-all"],["-nocase"],["-all","-nocase"],["-start","1"],["-start","end"],["-all","-start","1"]]
def hx(s): return s.encode().hex()
print("proc hx {s} {binary decode hex $s}")
for i in range(count):
    p=random.choice(pats); s=random.choice(texts); sp=random.choice(specs); fl=random.choice(flagsets)
    f=" ".join(fl)
    if random.random()<0.7:
        print(f'''set w SENT
set c [catch {{regsub {f} -- [hx "{hx(p)}"] [hx "{hx(s)}"] [hx "{hx(sp)}"] w}} r]
if {{$c}} {{puts "R{i} ERR<$r>"}} else {{puts "R{i} OK<[list $r $w]>"}}''')
    else:
        print(f'''set c [catch {{regsub {f} -- [hx "{hx(p)}"] [hx "{hx(s)}"] [hx "{hx(sp)}"]}} r]
if {{$c}} {{puts "R{i} ERR<$r>"}} else {{puts "R{i} OK<$r>"}}''')
