#!/usr/bin/env bash
set -euo pipefail

# Publish an existing draft release for name-route.
# Usage: ./scripts/publish_release.sh <version>
# Example: ./scripts/publish_release.sh v0.4.0

VERSION="${1:-}"

if [[ -z "$VERSION" ]]; then
  echo "Usage: $0 <version>" >&2
  echo "Example: $0 v0.4.0" >&2
  exit 1
fi

if [[ ! "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Error: version must match vX.Y.Z (e.g. v0.4.0)" >&2
  exit 1
fi

REPO="SpreadWorks/name-route"

if ! gh release view "$VERSION" >/dev/null 2>&1; then
  echo "Error: release $VERSION does not exist" >&2
  exit 1
fi

IS_DRAFT=$(gh release view "$VERSION" --json isDraft --jq '.isDraft')
if [[ "$IS_DRAFT" != "true" ]]; then
  echo "Error: release $VERSION is not a draft" >&2
  exit 1
fi

echo "==> Publishing release $VERSION"
gh release edit "$VERSION" --draft=false

echo "==> Release $VERSION published!"
echo "    https://github.com/$REPO/releases/tag/$VERSION"
