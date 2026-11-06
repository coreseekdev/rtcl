#!/usr/bin/env python3
"""生成 -nocase Unicode 大小写折叠随机差分语料（确定性 seed）。

覆盖 é/É、ä/Ä、ß/ẞ、Kelvin K、İ/ı、Σ/σ/ς、中 等字母在
literal/类/范围/POSIX 类下与 -nocase、-all 的组合。
pattern/text 以 UTF-8 hex 传入（binary decode 后按字节字符语义，
与 tclsh 一致）。

用法:
  python3 specs/regex/gen-fold-fuzz.py [seed] [count] > /tmp/foldfuzz.tcl
  tclsh /tmp/foldfuzz.tcl > o.txt
  ./target/release/rtcl -f /tmp/foldfuzz.tcl > r.txt
  diff o.txt r.txt
"""
import random, sys
seed = int(sys.argv[1]) if len(sys.argv) > 1 else 20261117
count = int(sys.argv[2]) if len(sys.argv) > 2 else 5000
random.seed(seed)
letters = ["a","A","é","É","ä","Ä","ö","Ö","ü","Ü","ß","ẞ","k","K","\u212a","i","I","İ","ı","中","Σ","σ","ς"]
texts = []
for a in letters[:16]:
    texts += [a, a+"a", "a"+a, a+a, a+"中"]
patterns = ["a","A","é","É","ä","Ä","ß","ẞ","k","K","\u212a","i","I","İ","ı","Σ","σ","ς","中",
            "[a]","[A]","[é]","[É]","[ä-ö]","[a-z]","[^a]","[^é]","é+","(é|É)","é?","é*",
            r"\w",r"\W","[[:alpha:]]",".",".."]
def hx(s): return s.encode().hex()
print("proc hx {s} {binary decode hex $s}")
for i in range(count):
    p=random.choice(patterns); s=random.choice(texts)
    flags=["-nocase"] if random.random()<0.85 else []
    if random.random()<0.3: flags.append("-all")
    if random.random()<0.5:
        f=" ".join(flags+["-inline","-indices"])
        print(f'''set c [catch {{regexp {f} -- [hx "{hx(p)}"] [hx "{hx(s)}"]}} r]
if {{$c}} {{puts "F{i} ERR<$r>"}} else {{puts "F{i} OK<$r>"}}''')
    else:
        f=" ".join(flags)
        print(f'''set w SENT; set g SENT
set c [catch {{regexp {f} -- [hx "{hx(p)}"] [hx "{hx(s)}"] w g}} e]
if {{$c}} {{puts "F{i} ERR<$e>"}} else {{puts "F{i} OK<[list $w $g]>"}}''')
