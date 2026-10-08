#!/usr/bin/env bash
# judge/lint-census: clippy warning census for the workspace
#
# Runs `cargo clippy --workspace --message-format=json`, tallies every warning
# by (lint code, file:line, message) and prints a count line at the end.
# Exit 0 iff clippy ran; "总计: N" is the number of distinct warnings.
#
# Note: after `| .message` the jq context is the diagnostic object itself, so
# the fields are .code.code / .spans[0] / .message — NOT .message.* (that
# re-indexes the rendered string and every warning is lost to a jq error).
#
# Usage:
#   judge/lint-census.sh         # print census, keep full detail in /tmp/lint-census.txt
#
# Burndown: each quality task re-runs this before/after to track progress
# toward clippy-zero.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo clippy --workspace --message-format=json 2>/dev/null \
  | jq -r 'select(.reason == "compiler-message")
      | .message // empty
      | select(.level == "warning")
      | [.code.code // "adhoc", .spans[0].file_name // "?", .spans[0].line_start // 0, (.message | gsub("\n"; " "))]
      | @tsv' \
  | sort | uniq -c | sort -rn | tee /tmp/lint-census.txt
echo "总计: $(grep -c '^' /tmp/lint-census.txt)"
