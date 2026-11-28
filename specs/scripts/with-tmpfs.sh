#!/usr/bin/env bash
# Run a command with a private, empty /tmp.
#
# Some hosts mount /tmp as a tmpfs with a per-user quota; a full shared /tmp
# makes tests that create temp files fail with "Disk quota exceeded" even
# though the code under test is fine. This wrapper gives the command its own
# tmpfs at /tmp via a user namespace (bubblewrap), leaving the host /tmp
# untouched.
#
# Usage:
#   specs/scripts/with-tmpfs.sh bash -c 'tclsh ...'
#   systemd-run --user --scope -p MemoryMax=2G --quiet \
#       specs/scripts/with-tmpfs.sh python3 specs/scripts/audit-91.py
#
# Requires bwrap(1) and unprivileged user namespaces. NSTMP_SIZE (default 4G)
# controls the private tmpfs size.
set -euo pipefail

size="${NSTMP_SIZE:-4G}"

exec bwrap \
    --dev-bind / / \
    --tmpfs /tmp \
    --chdir "$PWD" \
    -- \
    env TMPDIR=/tmp \
    "$@"
