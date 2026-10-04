#!/usr/bin/env bash
# What gets published is HEAD: every change outside probes/ must be committed,
# or the published branch is still the one the first review sent back.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"
echo "HEAD: $(git rev-parse --short HEAD)"
echo "committed tree at HEAD:"
echo "  admin-aggregator calls require_api_key: $(git show HEAD:crates/bcr-wdc-admin-aggregator/src/lib.rs | grep -c 'require_api_key(')"
echo "  core-service treasury setting: $(git show HEAD:crates/bcr-wdc-core-service/src/config.rs | grep -oE 'treasury[a-z_]*_url')"
echo "  wallet-aggregator treasury setting: $(git show HEAD:crates/bcr-wdc-wallet-aggregator/src/lib.rs | grep -m1 -oE 'treasury[a-z_]*_url')"
echo "  Dockerfiles with EXPOSE 3339: $(for s in core mint quote treasury; do git show HEAD:docker/${s}-service/Dockerfile; done | grep -c 'EXPOSE 3339')"
DIRTY="$(git status --porcelain --untracked-files=no -- . ':(exclude)probes')"
if [ -n "${DIRTY}" ]; then
  echo "${DIRTY}"
  git diff --stat -- . ':(exclude)probes' | tail -1
  echo "FAIL: tracked changes outside probes/ are not committed"
  exit 1
fi
echo PASS
