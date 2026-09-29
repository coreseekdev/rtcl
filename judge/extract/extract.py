#!/usr/bin/env python3
"""Extract deterministic, self-contained cases from Tcl's official tcltest
suite (.refer/tcl/tests/*.test) into judge corpus scripts.

Strategy:
  1. Tokenize each .test file with a minimal Tcl parser.
  2. Recognize `test` commands in two shapes:
       old:  test <name> <desc> {<body>} {<result>}
       new:  test <name> <desc> -body {<body>} -result {<result>} [-returnCodes ok|error]
     Anything more complex (constraints, -setup/-cleanup, -match glob/regexp,
     -output/-errorOutput, quoted/body substitution, unknown options) is skipped.
  3. Blacklist bodies touching IO/clock/random/environment (conservative regexes).
  4. Emit judge/corpus/gen/gen_<base>.tcl where each case prints one line:
       PASS, or "FAIL <name>".
  5. Verify: run every gen file twice under tclsh; keep only cases that print
     PASS both times with exit code 0 (drops cases needing file-local helpers
     or harboring nondeterminism).

Usage: python3 judge/extract/extract.py
Writes: judge/corpus/gen/*.tcl, judge/extract/stats.json
"""
import json
import os
import re
import subprocess
import sys
from collections import Counter

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TESTS = os.path.join(ROOT, ".refer", "tcl", "tests")
OUT = os.path.join(ROOT, "judge", "corpus", "gen")
STATS = os.path.join(ROOT, "judge", "extract", "stats.json")

# ---------------------------------------------------------------- tokenizer

class Word:
    __slots__ = ("kind", "raw")
    def __init__(self, kind, raw):
        self.kind = kind  # 'brace' | 'quote' | 'bare'
        self.raw = raw    # inner raw text (brace/quote) or full text (bare)

def skip_braces(s, i):
    """s[i] == '{'; return index past matching '}' or None."""
    n = len(s); j = i + 1; depth = 1
    while j < n:
        c = s[j]
        if c == "\\":
            j += 2
        elif c == "{":
            depth += 1; j += 1
        elif c == "}":
            depth -= 1; j += 1
            if depth == 0:
                return j
        else:
            j += 1
    return None

def skip_cmdsubst(s, i):
    """s[i] == '['; return index past matching ']' or None."""
    n = len(s); j = i + 1; depth = 1
    while j < n:
        c = s[j]
        if c == "\\":
            j += 2
        elif c == "[":
            depth += 1; j += 1
        elif c == "]":
            depth -= 1; j += 1
            if depth == 0:
                return j
        elif c == "{":
            j = skip_braces(s, j)
            if j is None:
                return None
        elif c == '"':
            k = j + 1
            while k < n:
                if s[k] == "\\":
                    k += 2
                elif s[k] == '"':
                    break
                elif s[k] == "[":
                    k2 = skip_cmdsubst(s, k)
                    if k2 is None:
                        return None
                    k = k2
                else:
                    k += 1
            if k >= n:
                return None
            j = k + 1
        else:
            j += 1
    return None

def parse_word(s, i):
    """Return (Word, next_index) or (None, next_index) on scan failure."""
    n = len(s)
    if s[i] == "{":
        j = skip_braces(s, i)
        if j is None:
            return None, n
        return Word("brace", s[i + 1:j - 1]), j
    if s[i] == '"':
        j = i + 1
        while j < n:
            c = s[j]
            if c == "\\":
                j += 2
            elif c == '"':
                return Word("quote", s[i + 1:j]), j + 1
            elif c == "[":
                k = skip_cmdsubst(s, j)
                if k is None:
                    return None, n
                j = k
            else:
                j += 1
        return None, n
    j = i
    while j < n:
        c = s[j]
        if c == "\\":
            j += 2
        elif c in " \t\r\n;":
            break
        elif c == "[":
            k = skip_cmdsubst(s, j)
            if k is None:
                return None, n
            j = k
        else:
            j += 1
    return Word("bare", s[i:j]), j

def parse_script(s):
    """Split s into commands; each command is a list of Words.
    Unparseable commands are yielded as None."""
    cmds = []
    i, n = 0, len(s)
    while i < n:
        # skip separators / comments
        while i < n:
            c = s[i]
            if c in " \t\r":
                i += 1
            elif c == "\\" and i + 1 < n and s[i + 1] == "\n":
                i += 2
            elif c in "\n;":
                i += 1
            elif c == "#":
                while i < n:
                    if s[i] == "\\" and i + 1 < n:
                        i += 2
                    elif s[i] == "\n":
                        break
                    else:
                        i += 1
            else:
                break
        if i >= n:
            break
        words = []
        ok = True
        while i < n:
            c = s[i]
            if c in " \t\r":
                i += 1
                continue
            if c in "\n;":
                i += 1
                break
            if c == "\\" and i + 1 < n and s[i + 1] == "\n":
                i += 2
                continue
            w, i = parse_word(s, i)
            if w is None:
                ok = False
                # resync: skip to next newline
                while i < n and s[i] != "\n":
                    i += 1
                break
            words.append(w)
            if w.kind == "brace" and i < n and s[i] not in " \t\r\n;":
                ok = False  # garbage after closing brace
                while i < n and s[i] != "\n":
                    i += 1
                break
        cmds.append(words if (ok and words) else None)
    return cmds

# ---------------------------------------------------------------- filters

NAME_RE = re.compile(r"^[\w.:-]+$")
SIMPLE_BARE_RE = re.compile(r'^[^$\\\[\]"{}\s;]*$')

BLACKLIST_CMDS = (
    "clock after socket exec open close gets read puts file glob pwd cd source "
    "load unload interp encoding fconfigure fcopy fileevent filevents vwait "
    "update coroutine yield yieldto zlib package pid exit time history "
    "dde registry consoleexit console tkcon safe base64 http"
).split()
BL_CMD_RE = re.compile(
    r"(?:^|[\[;{])\s*(?:::)?(?:" + "|".join(BLACKLIST_CMDS) + r")\b", re.M)

BLACKLIST_IDENT = [
    r"\brand\s*\(", r"\bsrand\b", r"\btcl_platform\b", r"\btcl_precision\b",
    r"\btcl_interactive\b", r"(?:\$|::)env\b", r"\benv\(", r"\bauto_path\b",
    r"\btcltest", r"\bmakeFile\b", r"\bremoveFile\b", r"\bmakeDirectory\b",
    r"\bremoveDirectory\b", r"\bviewFile\b", r"\btestConstraint\b",
    r"\bcleanupTests\b", r"\btest[A-Z]", r"\bparray\b", r"\b__judge_",
    r"\bunknown_cmd_handler\b",
    r"\binfo\s+(?:patchlevel|tclversion|hostname|nameofexecutable|"
    r"sharedlibextension|cmdcount|script|library|loaded|tests)\b",
]
BL_IDENT_RES = [re.compile(p) for p in BLACKLIST_IDENT]

def blacklisted(body):
    if BL_CMD_RE.search(body):
        return True
    return any(r.search(body) for r in BL_IDENT_RES)

# ---------------------------------------------------------------- extraction

def first_cmd(body):
    m = re.match(r"\s*(?:::)?([\w:]+)", body)
    return m.group(1) if m else "?"

def extract_file(path):
    """Return (cases, stats). cases = list of dicts."""
    with open(path, encoding="utf-8", errors="surrogateescape") as f:
        text = f.read()
    cmds = parse_script(text)
    cases = []
    stats = Counter()
    for words in cmds:
        if words is None:
            stats["unparseable-cmd"] += 1
            continue
        w0 = words[0]
        head = w0.raw if w0.kind in ("bare", "quote") else None
        if head not in ("test", "tcltest::test", "::tcltest::test"):
            continue
        stats["found"] += 1
        if len(words) < 4:
            stats["skip-shape"] += 1
            continue
        name_w = words[1]
        name = name_w.raw
        if not NAME_RE.match(name):
            stats["skip-name"] += 1
            continue
        rest = words[3:]
        body = result = None
        expect_code = 0
        if rest and rest[0].kind == "bare" and rest[0].raw.startswith("-"):
            # new format: option/value pairs
            if len(rest) % 2 != 0:
                stats["skip-shape"] += 1
                continue
            opts = {}
            bad = False
            for k in range(0, len(rest), 2):
                key = rest[k].raw
                if rest[k].kind != "bare" or not key.startswith("-"):
                    bad = True
                    break
                opts.setdefault(key, []).append(rest[k + 1])
            if bad:
                stats["skip-shape"] += 1
                continue
            skip_reason = None
            for o in ("-constraints", "-setup", "-cleanup", "-output",
                      "-errorOutput", "-not", "-errorCode", "-k"):
                if o in opts:
                    skip_reason = "skip-opt" + o
                    break
            if skip_reason is None and "-match" in opts:
                mv = opts["-match"][0]
                if not (mv.kind in ("bare", "brace") and mv.raw == "exact"):
                    skip_reason = "skip-opt-match"
            if skip_reason is None and "-body" not in opts:
                skip_reason = "skip-shape"
            if skip_reason is None:
                bw = opts["-body"][0]
                if bw.kind != "brace":
                    skip_reason = "skip-body-subst"
                else:
                    body = bw.raw
            if skip_reason is None:
                rw = opts["-result"][0] if "-result" in opts else None
                if rw is None:
                    result = ""
                elif rw.kind == "brace":
                    result = rw.raw
                elif rw.kind == "bare" and SIMPLE_BARE_RE.match(rw.raw):
                    result = rw.raw
                else:
                    skip_reason = "skip-result-subst"
            if skip_reason is None and "-returnCodes" in opts:
                rc = opts["-returnCodes"][0]
                if rc.kind in ("bare", "brace") and rc.raw in ("ok", "error"):
                    expect_code = 0 if rc.raw == "ok" else 1
                else:
                    skip_reason = "skip-returncodes"
            if skip_reason is None:
                known = {"-body", "-result", "-returnCodes", "-match", "-testlevel"}
                unk = [o for o in opts if o not in known]
                if unk:
                    skip_reason = "skip-opt-unknown"
            if skip_reason:
                stats[skip_reason] += 1
                continue
        else:
            # old format: body result [only]
            if len(rest) != 2:
                stats["skip-shape"] += 1
                continue
            bw, rw = rest
            if bw.kind != "brace":
                stats["skip-body-subst"] += 1
                continue
            if rw.kind == "brace":
                result = rw.raw
            elif rw.kind == "bare" and SIMPLE_BARE_RE.match(rw.raw):
                result = rw.raw
            else:
                stats["skip-result-subst"] += 1
                continue
            body = bw.raw
        if blacklisted(body):
            stats["skip-blacklist"] += 1
            continue
        stats["extracted"] += 1
        cases.append({
            "name": name, "body": body, "result": result,
            "expect": expect_code, "cmd": first_cmd(body),
        })
    return cases, stats

# ---------------------------------------------------------------- emission

def emit_case(case):
    name = case["name"]
    lines = []
    desc = " cmd=" + case["cmd"]
    lines.append("# case: %s%s" % (name, desc))
    lines.append("set __judge_c [catch {%s} __judge_r]" % case["body"])
    lines.append(
        'if {$__judge_c == %d && $__judge_r eq {%s}} {puts PASS} else {puts "FAIL %s"}'
        % (case["expect"], case["result"], name))
    return "\n".join(lines)

def emit_file(cases):
    out = ["# generated by judge/extract/extract.py -- do not edit"]
    for c in cases:
        out.append(emit_case(c))
    return "\n".join(out) + "\n"

def run_oracle(path):
    p = subprocess.run(["tclsh", path], capture_output=True)
    out = p.stdout.decode("utf-8", errors="surrogateescape")
    return p.returncode, out.split("\n")

def aligned(cases, code, lines):
    """Output is aligned iff tclsh exited 0 and every case produced exactly
    its one protocol line (PASS / FAIL <name>) and nothing else."""
    if code != 0 or len(lines) != len(cases) + 1 or lines[-1] != "":
        return False
    return all(l == "PASS" or l == "FAIL " + c["name"]
               for c, l in zip(cases, lines))

def verify_cases(cases, tmp_path):
    """Keep only cases that print PASS under tclsh, twice. Bisects around
    cases that crash the interpreter or desync the line protocol."""
    if not cases:
        return []
    with open(tmp_path, "w", encoding="utf-8", errors="surrogateescape") as f:
        f.write(emit_file(cases))
    code1, lines1 = run_oracle(tmp_path)
    code2, lines2 = run_oracle(tmp_path)
    if aligned(cases, code1, lines1) and aligned(cases, code2, lines2):
        return [c for c, l1, l2 in zip(cases, lines1, lines2)
                if l1 == "PASS" and l2 == "PASS"]
    if len(cases) == 1:
        return []
    mid = len(cases) // 2
    return (verify_cases(cases[:mid], tmp_path)
            + verify_cases(cases[mid:], tmp_path))

# ---------------------------------------------------------------- main

def main():
    os.makedirs(OUT, exist_ok=True)
    # clean previous generated files
    for f in os.listdir(OUT):
        if f.endswith(".tcl"):
            os.remove(os.path.join(OUT, f))
    all_stats = {}
    grand = Counter()
    for fname in sorted(os.listdir(TESTS)):
        if not fname.endswith(".test"):
            continue
        base = fname[:-5]
        cases, stats = extract_file(os.path.join(TESTS, fname))
        if not cases:
            all_stats[base] = dict(stats)
            grand.update(stats)
            continue
        tmp_path = os.path.join(OUT, "gen_%s.tcl" % base)
        keep = verify_cases(cases, tmp_path)
        stats["verify-dropped"] = len(cases) - len(keep)
        if keep:
            with open(tmp_path, "w", encoding="utf-8", errors="surrogateescape") as f:
                f.write(emit_file(keep))
        else:
            os.remove(tmp_path)
        stats["kept"] = len(keep)
        all_stats[base] = dict(stats)
        grand.update(stats)
    with open(STATS, "w") as f:
        json.dump({"per_file": all_stats, "total": dict(grand)}, f, indent=1)
    print("total:", dict(grand))
    gen_files = sorted(os.listdir(OUT))
    print("gen files: %d" % len(gen_files))
    for base in sorted(all_stats):
        st = all_stats[base]
        if st.get("kept"):
            print("  %-24s kept=%-5d extracted=%-5d dropped=%d" %
                  (base, st["kept"], st.get("extracted", 0),
                   st.get("verify-dropped", 0)))

if __name__ == "__main__":
    main()
