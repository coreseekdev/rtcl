import subprocess

def tclsh(script):
    return subprocess.run(['tclsh'], input=script, capture_output=True, text=True).stdout

def oracle_hist(keys):
    out = tclsh(f"puts [dict info [dict create {keys}]]")
    lines = out.splitlines()
    buckets = int(lines[0].split(", ")[1].split()[0])
    dist = {}
    for l in lines[1:11]:
        parts = l.split(": ")
        label = parts[0].split("with ")[1].split(" entries")[0]
        k = 10 if "or more" in label else int(label)
        dist[k] = int(parts[1])
    return buckets, dist

def h9(s):
    h = 0
    for c in s.encode():
        h = (h + (h<<3) + c) & 0xFFFFFFFF
    return h
def h33(s):
    h = 0
    for c in s.encode():
        h = (h*33 + c) & 0xFFFFFFFF
    return h

def simulate(keys, size0, hf, mult=4):
    size = size0
    rebuild = size * 3
    buckets = [[] for _ in range(size)]
    order = []  # iteration order (oldest-first per Tcl walk? we compute fresh below)
    for k in keys:
        idx = hf(k) % size
        if any(e == k for e in buckets[idx]):
            continue
        buckets[idx].insert(0, k)
        if sum(len(b) for b in buckets) >= rebuild:
            # Tcl walks old buckets 0..N, each chain head->tail
            old_order = [k2 for b in buckets for k2 in b]
            newsize = size * mult
            buckets = [[] for _ in range(newsize)]
            for k2 in old_order:
                buckets[hf(k2) % newsize].insert(0, k2)
            size, rebuild = newsize, newsize * 3
    dist = {}
    for b in buckets:
        d = min(len(b), 10)
        dist[d] = dist.get(d, 0) + 1
    return size, dist

for n in [3, 5, 9]:
    keys = [f"k{i}" for i in range(1, n+1)]
    arg = " ".join(f"{k} 1" for k in keys)
    ob, od = oracle_hist(" ".join(f"{k} 1" for k in keys))
    print(f"n={n} oracle: size={ob} dist={dict(sorted(od.items()))}")
    for name, hf in [("h9", h9), ("h33", h33)]:
        sb, sd = simulate(keys, 4, hf)
        print(f"   {name}: size={sb} dist={dict(sorted(sd.items()))}")
