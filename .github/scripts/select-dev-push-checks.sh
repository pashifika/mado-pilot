#!/usr/bin/env bash
set -euo pipefail

# Only a confirmed same-repository release PR for this exact push can replace
# the post-merge checks. Missing metadata and lookup failures keep them enabled.
skip_checks=false
if [[ "${GITHUB_REF_NAME}" =~ ^dev/[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  if pulls="$(gh api --method GET "repos/${GITHUB_REPOSITORY}/pulls" \
    --paginate --slurp \
    -f state=open \
    -f base=main \
    -f head="${GITHUB_REPOSITORY_OWNER}:${GITHUB_REF_NAME}" \
    -f per_page=100)"; then
    if matches="$(jq -r \
      --arg repository "${GITHUB_REPOSITORY}" \
      --arg branch "${GITHUB_REF_NAME}" \
      --arg sha "${GITHUB_SHA}" \
      'any(.[][];
        .state == "open" and
        .base.ref == "main" and
        .base.repo.full_name == $repository and
        .head.repo.full_name == $repository and
        .head.ref == $branch and
        .head.sha == $sha)' <<< "${pulls}")"; then
      if [[ "${matches}" == "true" ]]; then
        skip_checks=true
      fi
    else
      echo "::warning::Release PR response could not be evaluated; running dev push checks."
    fi
  else
    echo "::warning::Release PR lookup failed; running dev push checks."
  fi
fi

if [[ "${skip_checks}" == "true" ]]; then
  echo "An open release PR covers this exact dev head; skipping duplicate push checks."
else
  echo "No covering release PR confirmed; running dev push checks."
fi
printf 'skip-checks=%s\n' "${skip_checks}" >> "${GITHUB_OUTPUT}"
