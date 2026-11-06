#!/usr/bin/env python3
"""生成 -expanded 空白/注释随机差分语料（确定性 seed）。

覆盖 ARE/BRE/ERE 下的空白、`#` 注释、转义空格、类内空格、
`(?x)` 前缀与 -expanded 命令开关组合。

用法:
  python3 specs/regex/gen-expanded-fuzz.py [seed] [count] > /tmp/expfuzz.tcl
  tclsh /tmp/expfuzz.tcl > o.txt
  ./target/release/rtcl -f /tmp/expfuzz.tcl > r.txt
  diff o.txt r.txt
"""
import random, sys
seed = int(sys.argv[1]) if len(sys.argv) > 1 else 20261119
count = int(sys.argv[2]) if len(sys.argv) > 2 else 3000
random.seed(seed)
atoms=["a","b","[ab]","[^a]","(a)","(?:a)","a*","a+","a?","a{1,2}","\\.","\\\\","-","#"," "]
ws=[" ","  ","\t","#c\n"," #x\n ","\n"]
texts=["ab","a b","a\tb","a#c\nb","a#c","ba","","a\nb"]
def hx(s): return s.encode().hex()
print("proc hx {s} {binary decode hex $s}")
for i in range(count):
    n=random.randint(1,4); parts=[]
    for j in range(n):
        parts.append(random.choice(atoms))
        if j<n-1 and random.random()<0.7: parts.append(random.choice(ws))
    p="".join(parts)
    if random.random()<0.2: p="(?x)"+p
    if random.random()<0.15: p="(?b)"+p
    if random.random()<0.15: p="(?e)"+p
    s=random.choice(texts)
    fl=["-expanded"] if random.random()<0.8 else []
    f=" ".join(fl+["-inline","-indices"])
    print(f'''set c [catch {{regexp {f} -- [hx "{hx(p)}"] [hx "{hx(s)}"]}} r]
if {{$c}} {{puts "E{i} ERR<$r>"}} else {{puts "E{i} OK<$r>"}}''')
