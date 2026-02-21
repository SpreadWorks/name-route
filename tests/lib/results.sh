#!/usr/bin/env bash
# Shared results collection and display functions for E2E tests.
# Source this file; do not execute directly.

set -euo pipefail

RESULTS_DIR=""

# Create a temporary directory for test output files.
init_results_dir() {
    RESULTS_DIR=$(mktemp -d /tmp/nameroute-results-XXXXXX)
}

# Remove the results directory.
cleanup_results_dir() {
    [ -n "$RESULTS_DIR" ] && rm -rf "$RESULTS_DIR"
    RESULTS_DIR=""
}

# Collect PG/MySQL result from an output file.
# Usage: collect_result <outfile>
# Prints two lines: the PG result and MySQL result (PASS, FAIL:..., or ERROR).
collect_result() {
    local outfile="$1"
    local pg my
    pg=$(grep "^PG:" "$outfile" 2>/dev/null | head -1 | cut -d: -f2 || true)
    my=$(grep "^MySQL:" "$outfile" 2>/dev/null | head -1 | cut -d: -f2 || true)
    [ -z "$pg" ] && pg="ERROR"
    [ -z "$my" ] && my="ERROR"
    echo "$pg"
    echo "$my"
}
