#!/usr/bin/env bash
set -euo pipefail

pr_number="${1:?PR number is required}"
head_ref="${2:?Source branch is required}"
: "${GH_REPO:?GH_REPO is required}"
preview_tag="pre-${head_ref}"

if gh release view "$preview_tag" --json id >/dev/null 2>&1; then
  echo "Deleting preview release $preview_tag."
  if ! gh release delete "$preview_tag" --yes; then
    if gh release view "$preview_tag" --json id >/dev/null 2>&1; then
      echo "::error::Failed to delete preview release $preview_tag."
      exit 1
    fi
    echo "Preview release $preview_tag was already removed."
  fi
else
  echo "Preview release $preview_tag does not exist; skipping."
fi

if gh api "repos/$GH_REPO/git/ref/tags/$preview_tag" >/dev/null 2>&1; then
  echo "Deleting preview tag $preview_tag."
  if ! gh api --method DELETE "repos/$GH_REPO/git/refs/tags/$preview_tag" >/dev/null; then
    if gh api "repos/$GH_REPO/git/ref/tags/$preview_tag" >/dev/null 2>&1; then
      echo "::error::Failed to delete preview tag $preview_tag."
      exit 1
    fi
    echo "Preview tag $preview_tag was already removed."
  fi
else
  echo "Preview tag $preview_tag does not exist; skipping."
fi

run_ids="$(gh api --paginate "repos/$GH_REPO/actions/workflows/pr-preview.yml/runs" \
  -f event=pull_request -f branch="$head_ref" -f per_page=100 --jq '.workflow_runs[].id')"
for run_id in $run_ids; do
  if ! artifact_ids="$(gh api --paginate "repos/$GH_REPO/actions/runs/$run_id/artifacts" \
    -f per_page=100 --jq '.artifacts[].id')"; then
    if ! gh api "repos/$GH_REPO/actions/runs/$run_id" >/dev/null 2>&1; then
      echo "Preview run $run_id no longer exists; skipping its artifacts."
      continue
    fi
    echo "::error::Failed to list artifacts for preview run $run_id."
    exit 1
  fi
  for artifact_id in $artifact_ids; do
    echo "Deleting artifact $artifact_id from preview run $run_id."
    if ! gh api --method DELETE "repos/$GH_REPO/actions/artifacts/$artifact_id" >/dev/null; then
      if gh api "repos/$GH_REPO/actions/artifacts/$artifact_id" >/dev/null 2>&1; then
        echo "::error::Failed to delete artifact $artifact_id."
        exit 1
      fi
      echo "Artifact $artifact_id was already removed."
    fi
  done
done

echo "Removed preview release, tag, and build artifacts for PR #${pr_number}."
