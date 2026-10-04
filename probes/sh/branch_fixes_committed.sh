#!/usr/bin/env bash
# What gets published is HEAD: every change outside probes/ must be committed,
# or the published branch is still the one the first review sent back.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"
echo "HEAD: $(git rev-parse --short HEAD)"
DIRTY="$(git status --porcelain --untracked-files=no -- . ':(exclude)probes')"
if [ -n "${DIRTY}" ]; then
  echo "${DIRTY}"
  git diff --stat -- . ':(exclude)probes' | tail -1
  echo "FAIL: tracked changes outside probes/ are not committed"
  exit 1
fi
echo PASS
