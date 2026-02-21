#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

SUITES=("db-versions" "clients")

run_suite() {
    local suite="$1"
    echo ""
    echo "######################################################################"
    echo "  Suite: $suite"
    echo "######################################################################"
    echo ""
    bash "$SCRIPT_DIR/$suite/run_all.sh"
}

if [ $# -eq 0 ]; then
    # Run all suites
    EXIT_CODE=0
    for suite in "${SUITES[@]}"; do
        run_suite "$suite" || EXIT_CODE=1
    done
    exit $EXIT_CODE
fi

case "$1" in
    db-versions|clients)
        run_suite "$1"
        ;;
    *)
        echo "Usage: $0 [db-versions|clients]"
        echo ""
        echo "  No arguments: run all suites"
        echo "  db-versions:  run DB server version matrix tests"
        echo "  clients:      run client library matrix tests"
        exit 1
        ;;
esac
