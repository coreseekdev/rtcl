#!/usr/bin/env python3
"""生成 regexp/regsub 命令面随机差分语料（确定性 seed）。

覆盖开关组合（-nocase/-all/-inline/-indices/-expanded/-line*/-start）、
match vars、regsub 替换串、BRE/ERE/ARE 前缀、空匹配与 -start 越界。
pattern/string/spec 以 UTF-8 十六进制传入，输出行带 C<编号> 标签，
便于 tclsh 与 rtcl 逐例 diff。

用法（可选 seed/count）:
  python3 specs/regex/gen-cmd-fuzz.py [seed] [count] > /tmp/cmdfuzz.tcl
  tclsh /tmp/cmdfuzz.tcl > o.txt
  ./target/release/rtcl -f /tmp/cmdfuzz.tcl > r.txt
  diff o.txt r.txt
"""
import random, sys
seed = int(sys.argv[1]) if len(sys.argv) > 1 else 20261107
count = int(sys.argv[2]) if len(sys.argv) > 2 else 3500
random.seed(seed)
atoms = ["a","b",".","[ab]","[^a]","[a-c]",r"\d",r"\w",r"\s","(a)","(a|b)","(a*)","(a+)","a?","a*","a+","a{1,2}","a{0,2}","^","$",r"\y",r"\m",r"\M",r"\Y","(?=a)","(?:a)","(a)\\1","a*?",r"(a+?)(a*)","[[:alpha:]]","[[:digit:]]","[^a-c]","()","(a|)","a|b",r"\A",r"\Z"]
texts = ["", "a", "aa", "ab", "abc", "a\nb", " a ", "A", "1", "abab", "ba", "a b", "xa", "\n", "aaa", "a1b2", "\u00e9a", "ab\ncd", "A1", "a\tb", "\u00e9", "\r\n", "abc\n", "()"]
specs = ["X", "&", r"\1", r"[\1]", r"\0", r"\&", r"\\", "pre&post", r"\1\1", "", "a\\b", r"\2", r"&\1", r"x\1y"]
def pat():
    n=random.randint(1,4)
    p="".join(random.choice(atoms) for _ in range(n))
    if random.random()<0.1: p="***="+p
    if random.random()<0.08: p="(?b)"+p
    if random.random()<0.08: p="(?e)"+p
    return p
def flags(for_regsub=False):
    f=[]
    if random.random()<0.3: f.append("-nocase")
    if random.random()<0.35: f.append("-all")
    if random.random()<0.15: f.append("-expanded")
    if random.random()<0.2: f.append("-line")
    elif random.random()<0.2: f.append("-linestop")
    elif random.random()<0.2: f.append("-lineanchor")
    if random.random()<0.35:
        f += ["-start", random.choice(["0","1","2","end","end-1","-1","3"])]
    if not for_regsub and random.random()<0.3: f.append("-inline")
    if not for_regsub and "-inline" not in f and random.random()<0.3: f.append("-indices")
    return f
def hx(s): return s.encode().hex()
cases=[]
for i in range(count):
    p=pat(); s=random.choice(texts); f=flags()
    mode=random.choices(["inline","count","vars","indices","regsub","regsub_novar"],[3,2,2,1.5,2,1])[0]
    if mode in ("inline","count","vars","indices"):
        fl=list(f)
        if mode=="inline" and "-inline" not in fl: fl.append("-inline")
        if mode=="count":
            fl=[x for x in fl if x not in ("-inline","-indices")]
        if mode=="indices":
            fl=[x for x in fl if x!="-inline"]+["-indices"]
        if mode=="vars":
            fl=[x for x in fl if x not in ("-inline",)]
        fstr=" ".join(fl)
        if mode in ("inline","count"):
            cases.append(f'''set c [catch {{regexp {fstr} -- [hx "{hx(p)}"] [hx "{hx(s)}"]}} r]
if {{$c}} {{puts "ERR<$r>"}} else {{puts "OK<$r>"}}''')
        elif mode=="indices":
            cases.append(f'''set w SENT; set g1 SENT; set g2 SENT
set c [catch {{regexp {fstr} -- [hx "{hx(p)}"] [hx "{hx(s)}"] w g1 g2}} err]
if {{$c}} {{puts "ERR<$err>"}} else {{puts "OK<[list $w $g1 $g2]>"}}''')
        else:
            cases.append(f'''set w SENT; set g1 SENT; set g2 SENT
set c [catch {{regexp {fstr} -- [hx "{hx(p)}"] [hx "{hx(s)}"] w g1 g2}} err]
if {{$c}} {{puts "ERR<$err>"}} else {{puts "OK<[list $w $g1 $g2]>"}}''')
    else:
        spec=random.choice(specs)
        fl=flags(for_regsub=True)
        fstr=" ".join(fl)
        if mode=="regsub":
            cases.append(f'''set w SENT
set c [catch {{regsub {fstr} -- [hx "{hx(p)}"] [hx "{hx(s)}"] [hx "{hx(spec)}"] w}} err]
if {{$c}} {{puts "ERR<$err>"}} else {{puts "OK<[list $err $w]>"}}''')
        else:
            cases.append(f'''set c [catch {{regsub {fstr} -- [hx "{hx(p)}"] [hx "{hx(s)}"] [hx "{hx(spec)}"]}} r]
if {{$c}} {{puts "ERR<$r>"}} else {{puts "OK<$r>"}}''')
print("proc hx {s} {binary decode hex $s}")
for i,c in enumerate(cases):
    print(c.replace('puts "', f'puts "C{i} '))
