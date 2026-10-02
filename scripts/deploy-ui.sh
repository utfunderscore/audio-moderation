#!/usr/bin/env bash

# Build and deploy the UI using the public outputs from the deployed Terraform state.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
PRIMARY_DIR="$(dirname -- "$(git -C "${ROOT_DIR}" rev-parse --path-format=absolute --git-common-dir)")"
TERRAFORM_DIR="${TERRAFORM_DIR:-${PRIMARY_DIR}/terraform}"

if [[ ! -f "${TERRAFORM_DIR}/terraform.tfstate" ]]; then
    printf 'No local Terraform state in %s; set TERRAFORM_DIR to the deployed state directory.\n' "${TERRAFORM_DIR}" >&2
    exit 1
fi
if [[ -z "${CLOUDFLARE_ACCOUNT_ID:-}" ]]; then
    printf 'Set CLOUDFLARE_ACCOUNT_ID to the Cloudflare account that owns the UI.\n' >&2
    exit 1
fi

export AWS_PROFILE=admin TERRAFORM_DIR
UI_ENV_DIR="${ROOT_DIR}/ui" "${ROOT_DIR}/ui/scripts/sync-deployment-env.sh"

# Vite gives existing process variables precedence over the synchronized env file.
unset VITE_TURNSTILE_SITE_KEY VITE_API_ENDPOINT VITE_TASK_EVENTS_ENDPOINT
cd "${ROOT_DIR}/ui"
npm run deploy
