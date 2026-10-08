#!/usr/bin/env bash
# judge/panic-census: production-surface panic-site census
#
# Counts .unwrap() / .expect( / panic! / unreachable! / todo! in crates/*/src,
# stopping each file at its first test gate and skipping comment/doc lines, so
# only reachable-from-production sites are counted.
#
# Output format: <file>:<line>: <kind>
#   kind ∈ {unwrap, expect, panic, unreachable, todo}
#   (first match on a line wins, in that order)
#
# Reliability notes (both shapes have bitten before):
#   - FNR==1 resets the per-file test flag; without that reset the first
#     test-carrying file poisons every later file and the census under-reports.
#   - Test gates are not always `#[cfg(test)]`: this repo also uses
#     `#[cfg(all(test, feature = "io"))]` and friends. Any cfg(...test...)
#     opens the test region.
#   - Sibling test modules (misc_tests.rs etc.) are pulled in via a test-gated
#     `#[path = "..."]` in their parent and carry no gate of their own; they
#     are resolved and excluded entirely.
#
# Usage:
#   judge/panic-census.sh        # print census, keep full list in /tmp/panic-census.txt
set -euo pipefail
cd "$(dirname "$0")/.."

srcs=$(find crates -path '*/src/*.rs' ! -path '*/tests/*' | sort)

# Resolve test-gated `#[path]` includes (test code living in marker-less files).
excl=$(mktemp)
trap 'rm -f "$excl"' EXIT
for f in $srcs; do
  awk -v dir="$(dirname "$f")" '
    /^[[:space:]]*(\/\/|[\/!][\/!])/ { next }      # 跳过注释/doc
    /#\[cfg\(.*test.*\)\]/ { g = 1; next }         # a test gate arms the look-ahead
    g && /^#\[path[[:space:]]*=[[:space:]]*"[^"]+"\]/ {
      match($0, /"[^"]*"/)
      print dir "/" substr($0, RSTART + 1, RLENGTH - 2)
      g = 0; next
    }
    /^[[:space:]]*$/ { if (g) g++; next }          # tolerate blanks between attrs
    { g = 0 }
  ' "$f"
done > "$excl"

for f in $srcs; do
  grep -Fxq "$f" "$excl" && continue
  awk -v file="$f" '
    FNR == 1 { t = 0 }                             # 每个文件复位 test 标志
    /^[[:space:]]*(\/\/|[\/!][\/!])/ { next }      # 跳过注释/doc
    /#\[cfg\(.*test.*\)\]/ { t = 1 }
    t == 0 && /\.unwrap\(\)/ { print file ":" FNR ": unwrap"; next }
    t == 0 && /\.expect\(/   { print file ":" FNR ": expect"; next }
    t == 0 && /panic!/       { print file ":" FNR ": panic"; next }
    t == 0 && /unreachable!/{ print file ":" FNR ": unreachable"; next }
    t == 0 && /todo!\(/      { print file ":" FNR ": todo"; next }
  ' "$f"
done | tee /tmp/panic-census.txt
echo "总计: $(wc -l < /tmp/panic-census.txt)"
