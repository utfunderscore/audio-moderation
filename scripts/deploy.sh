#!/usr/bin/env bash

set -Eeuo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/aws-context.sh"
source "${SCRIPT_DIR}/lib/deployment-outputs.sh"
source "${SCRIPT_DIR}/deployment/config.sh"

usage() {
    cat <<'EOF'
Usage:
  AWS_PROFILE=admin ./scripts/deploy.sh preflight [options]
  AWS_PROFILE=admin ./scripts/deploy.sh deploy [options]

preflight validates prerequisites without deploying, decrypting secrets, applying
migrations, or contacting model endpoints. deploy runs preflight automatically,
publishes all eight images with one immutable tag, and applies infrastructure.

Options (also configurable through corresponding uppercase environment variables):
  --region REGION                        Default: eu-west-2
  --project-name NAME                    Default: audio-moderation
  --environment NAME                     Default: dev
  --tenant-id ID                         Default: default
  --database-parameter-name NAME         SecureString database URL parameter
  --turnstile-secret-parameter-name NAME  Terraform-managed Turnstile parameter
  --turnstile-allowed-hostnames NAMES     Required comma-separated exact hostnames
  --modal-token-id-parameter-name NAME    SecureString Modal token ID parameter
  --modal-token-secret-parameter-name NAME
  --transcription-endpoint-url URL       HTTPS transcription endpoint
  --modal-endpoint-url URL               HTTPS moderation base URL
  --modal-workspace-id ID                Modal workspace ID
  --enable-cloudflare-proxy              Publish APIs through Cloudflare
  --cloudflare-zone-name NAME            Default: utf.lol
  --public-api-domain-name NAME           Default: api-guard.utf.lol
  --task-events-domain-name NAME          Default: events-guard.utf.lol
  --image-tag TAG                        Default: git SHA plus UTC timestamp
  --auto-approve                         Skip Terraform approval prompts
  -h, --help

Deployment requires explicit approval. Never deploy concurrently from separate
worktrees: Terraform uses local state. See scripts/deployment/README.md.
EOF
}

ACTION="${1:-}"
case "${ACTION}" in
    ''|-h|--help) usage; exit 0 ;;
    preflight|deploy) shift ;;
    *) printf 'Unknown deployment command: %s\n' "${ACTION}" >&2; usage >&2; exit 2 ;;
esac
parse_deployment_options "$@"
configure_aws_context
configure_deployment
source "${SCRIPT_DIR}/deployment/preflight.sh"

case "${ACTION}" in
    preflight) deployment_preflight ;;
    deploy)
        source "${SCRIPT_DIR}/deployment/images.sh"
        source "${SCRIPT_DIR}/deployment/terraform.sh"
        deploy
        ;;
esac
