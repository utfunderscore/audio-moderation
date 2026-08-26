#!/usr/bin/env bash
# Deploy the production Granite gateway after the local validation gate passes.

set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v uv >/dev/null 2>&1; then
  printf '%s\n' "uv is required. Install it from https://docs.astral.sh/uv/." >&2
  exit 1
fi

cd "$project_root"

uv lock --check
uv run ruff format --check .
uv run ruff check .
uv run basedpyright
uv run pytest

cat <<'EOF'
Deploying socialguard_models.deployments.granite to Modal.

Required remote prerequisites:
  - Modal authentication is configured.
  - The socialguard-gateway-api secret includes SOCIALGUARD_GATEWAY_API_TOKEN.
  - The pinned Granite model is pre-fetched in the Modal Volume.
EOF

exec uv run modal deploy -m socialguard_models.deployments.granite
