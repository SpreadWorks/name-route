#!/usr/bin/env bash
set -euo pipefail

# Create a draft release for name-route.
# Usage: ./scripts/draft_release.sh <version>
# Example: ./scripts/draft_release.sh v0.4.0

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

# Ensure we're on main and clean
BRANCH=$(git branch --show-current)
if [[ "$BRANCH" != "main" ]]; then
  echo "Error: must be on main branch (currently on '$BRANCH')" >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Error: working tree is not clean" >&2
  git status --short
  exit 1
fi

# Check tag doesn't already exist
if git rev-parse "$VERSION" >/dev/null 2>&1; then
  echo "Error: tag $VERSION already exists" >&2
  exit 1
fi

echo "==> Creating tag $VERSION"
git tag "$VERSION"
git push origin main
git push origin "$VERSION"

echo "==> Waiting for Release workflow to complete..."
# Give GitHub a moment to register the tag-triggered workflow.
sleep 5
RUN_ID=$(gh run list --workflow=release.yml --branch="$VERSION" --limit=1 --json databaseId --jq '.[0].databaseId')

if [[ -z "$RUN_ID" ]]; then
  echo "Error: could not find workflow run for $VERSION" >&2
  echo "Check manually: gh run list --workflow=release.yml" >&2
  exit 1
fi

echo "    Workflow run: https://github.com/$REPO/actions/runs/$RUN_ID"
gh run watch "$RUN_ID" --exit-status || {
  echo "Error: workflow failed. Fix the issue, then clean up with:" >&2
  echo "  gh release delete $VERSION --yes" >&2
  echo "  git push origin --delete $VERSION" >&2
  echo "  git tag -d $VERSION" >&2
  exit 1
}

echo "==> Draft release created with build assets."
echo "    Test the draft assets, then publish with:"
echo "      ./scripts/publish_release.sh $VERSION"
echo "    https://github.com/$REPO/releases/tag/$VERSION"
