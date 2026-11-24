# crash-probes: script-triggered crash / hang / silent-wrong-result gate
#
# judge/crash-probes/*.tcl each pin one defect found by a code audit
# (specs/CODE-AUDIT-20261123.md): a script that used to abort the process
# (Rust panic, stack overflow, allocation failure — release is
# panic = "abort", so Tcl `catch` cannot intercept any of them), hang, or
# return a silently wrong result.
#
# A probe prints "ok <name>" per assertion (see _chk.tcl) and exits 1 on a
# mismatch; the gate requires exit code 0 within the time budget and no
# panic text, on both engines unless the probe carries "# gate: skip-tree".
#
# Probes run under a 3GB address-space cap so a regression aborts with
# "memory allocation ... failed" instead of eating the machine. tclsh
# handles every crash-class input with a normal result or a Tcl error —
# that is the bar.
#
# Usage:
#   bash judge/crash-gate.sh            # all probes
#   bash judge/crash-gate.sh b1 -v      # probes named b1*, show failures
#   exit 0 iff every probe passes.
# The wasm side has its own runner over the same classes of defect:
#   node judge/crash-wasm.mjs           # after crates/rtcl-wasm/build.sh
set -u
cd "$(dirname "$0")/.."

RTCL=./target/release/rtcl
VERBOSE=0
PREFIX=""
for a in "$@"; do
    case "$a" in
        -v) VERBOSE=1 ;;
        *) PREFIX="$a" ;;
    esac
done
[ ! -x "$RTCL" ] && { echo "crash-gate: $RTCL not found; cargo build --release first" >&2; exit 2; }

# A probe may carry "# gate: max-ms <n>" (default 20000): wall time above it
# counts as a hang, so a gate on "returns" also gates on "returns promptly".
budget_ms() { grep -oE '^# gate: max-ms [0-9]+' "$1" | grep -oE '[0-9]+' || true; }

# A probe may carry "# gate: env-nonutf8": it then runs in a scrubbed
# environment holding one variable that is not valid Unicode — the state
# has to exist before the interpreter is built, so no script can set it
# up itself (a13: `Interp::new()` used to panic on it).
run_one() {
    local f=$1 eng=$2 max out rc t0 t1 pre=()
    max=$(budget_ms "$f"); max=${max:-20000}
    grep -q '^# gate: env-nonutf8' "$f" && pre=(env -i "PATH=$PATH" $'RTCL_BAD=\xff\xfe')
    # Output goes to a file, not a command substitution: a child the
    # script left running inherits the interpreter's stdout and stderr,
    # and a `$(...)` reading those through a pipe only sees EOF once that
    # child is gone — the clock would then time the child, not the probe
    # (`exit` next to `open "|sleep 3"` measured 3s with the interpreter
    # gone in 30ms).
    local capture
    capture=$(mktemp) || exit 2
    t0=$(date +%s%N)
    "${pre[@]}" bash -c "ulimit -v 3000000; ulimit -c 0; timeout 30 $RTCL $eng -f '$f'" > "$capture" 2>&1; rc=$?
    t1=$(date +%s%N)
    out=$(head -c 200000 "$capture")
    rm -f "$capture"
    if [ "$rc" -eq 0 ] && [ $(( (t1 - t0) / 1000000 )) -le "$max" ] \
        && ! grep -qE 'panicked|stack overflow|memory allocation of' <<<"$out"; then
        return 0
    fi
    [ "$VERBOSE" -eq 1 ] && echo "--- $(basename "$f") [$eng] rc=$rc $(( (t1 - t0) / 1000000 ))ms" \
        && head -4 <<<"$out" | cut -c1-160 | sed 's/^/    /'
    return 1
}

pass=0; fail=0; failed=()
for f in judge/crash-probes/${PREFIX}*.tcl; do
    case "$(basename "$f")" in _*) continue ;; esac
    engines=("" "--no-bytecode")
    grep -q '^# gate: skip-tree' "$f" && engines=("")
    for eng in "${engines[@]}"; do
        if run_one "$f" "$eng"; then
            pass=$((pass + 1))
        else
            fail=$((fail + 1)); failed+=("$(basename "$f" .tcl)${eng:+(tree)}")
        fi
    done
done

echo "crash-gate: $((pass + fail)) runs, $pass pass, $fail fail"
if [ "$fail" -gt 0 ]; then
    printf '  FAIL %s\n' "${failed[@]}"
    exit 1
fi
