#!/usr/bin/env python3
"""生成反向引用匹配/捕获随机差分语料（确定性 seed）。

为避免 tclsh 8.6 回引回溯在随机嵌套量词下指数卡死，语法受限：
分组内容简单，回引只出现在已闭合组之后，不在外层量词里嵌套回引。
覆盖 -nocase/-all/-indices/match vars。

用法:
  python3 specs/regex/gen-backref-fuzz.py [seed] [count] > /tmp/breffuzz.tcl
  tclsh /tmp/breffuzz.tcl > o.txt
  ./target/release/rtcl -f /tmp/breffuzz.tcl > r.txt
  diff o.txt r.txt
"""
import random, sys
seed = int(sys.argv[1]) if len(sys.argv) > 1 else 20261122
count = int(sys.argv[2]) if len(sys.argv) > 2 else 5000
random.seed(seed)
groups=["(a)","(b)","(a*)","(a+)","(a|b)","(ab)","(a?)","((a))","(a|ab)","(.)","(a*b?)"]
texts=["","a","b","ab","aa","ba","abab","aaa","abb","baa","x"]
flagsets=[[],["-nocase"],["-all"],["-all","-nocase"],["-indices"],["-all","-inline","-indices"],["-inline","-indices"]]
def hx(s): return s.encode().hex()
print("proc hx {s} {binary decode hex $s}")
for i in range(count):
    n=random.randint(1,4); parts=[]; ng=0
    for _ in range(n):
        r=random.random()
        if r<0.45:
            parts.append(random.choice(groups)); ng+=1
        elif r<0.75 and ng>0:
            parts.append("\\%d"%random.randint(1,min(ng,9)))
        else:
            parts.append(random.choice(["a","b",".","a?","a*","[ab]"]))
    p="".join(parts)
    if random.random()<0.1: p="(?i)"+p
    s=random.choice(texts); f=" ".join(random.choice(flagsets))
    if random.random()<0.5:
        ff=(f+" -inline -indices").strip() if "-inline" not in f else f
        print(f'''set c [catch {{regexp {ff} -- [hx "{hx(p)}"] [hx "{hx(s)}"]}} r]
if {{$c}} {{puts "B{i} ERR<$r>"}} else {{puts "B{i} OK<$r>"}}''')
    else:
        ff=[x for x in f.split() if x!="-inline"]
        f2=" ".join(ff)
        print(f'''set w SENT; set g1 SENT; set g2 SENT
set c [catch {{regexp {f2} -- [hx "{hx(p)}"] [hx "{hx(s)}"] w g1 g2}} e]
if {{$c}} {{puts "B{i} ERR<$e>"}} else {{puts "B{i} OK<[list $w $g1 $g2]>"}}''')
