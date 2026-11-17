#!/usr/bin/env bash
set -euo pipefail

pr_number="${1:?PR number is required}"
head_ref="${2:?Source branch is required}"
: "${GH_REPO:?GH_REPO is required}"
preview_tag="pre-${head_ref}"

if gh release view "$preview_tag" --json id >/dev/null 2>&1; then
  gh release delete "$preview_tag" --yes
fi

if gh api "repos/$GH_REPO/git/ref/tags/$preview_tag" >/dev/null 2>&1; then
  gh api --method DELETE "repos/$GH_REPO/git/refs/tags/$preview_tag" >/dev/null
fi

run_ids="$(gh api --paginate "repos/$GH_REPO/actions/workflows/pr-preview.yml/runs" \
  -f event=pull_request -f branch="$head_ref" -f per_page=100 --jq '.workflow_runs[].id')"
for run_id in $run_ids; do
  artifact_ids="$(gh api --paginate "repos/$GH_REPO/actions/runs/$run_id/artifacts" \
    -f per_page=100 --jq '.artifacts[].id')"
  for artifact_id in $artifact_ids; do
    gh api --method DELETE "repos/$GH_REPO/actions/artifacts/$artifact_id" >/dev/null
  done
done

echo "Removed preview release, tag, and build artifacts for PR #${pr_number}."
