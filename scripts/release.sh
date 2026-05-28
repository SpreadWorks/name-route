#!/usr/bin/env bash
set -euo pipefail

echo "Error: scripts/release.sh is deprecated to avoid accidentally publishing releases." >&2
echo "" >&2
echo "Use one of:" >&2
echo "  ./scripts/draft_release.sh <version>    # create draft release and stop" >&2
echo "  ./scripts/publish_release.sh <version>  # publish an existing draft" >&2
exit 1
