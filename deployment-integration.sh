#!/usr/bin/env bash

# Compatibility only. New callers should use the scoped runners in scripts/.
set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "${ROOT_DIR}/scripts/lib/aws-context.sh"

usage() {
    cat <<'EOF'
Legacy compatibility entry point:
  AWS_PROFILE=admin ./deployment-integration.sh preflight [suite] [options]
  AWS_PROFILE=admin ./deployment-integration.sh deploy [options]
  AWS_PROFILE=admin ./deployment-integration.sh test <suite> [options]
  AWS_PROFILE=admin ./deployment-integration.sh all [suite] [options]

Prefer ./scripts/deploy.sh or ./scripts/test-deployed.sh; use --help on those
runners for scoped options. Legacy options are routed to the relevant workflow.
all validates suite prerequisites before deploying all eight Lambdas, then tests
the deployment (default suite: review-confirmation).
EOF
}

ACTION="${1:-}"
case "${ACTION}" in
    ''|-h|--help) usage; exit 0 ;;
    preflight|deploy|test|all) shift ;;
    *) printf 'Unknown command: %s\n' "${ACTION}" >&2; usage >&2; exit 2 ;;
esac
SUITE=""
case "${ACTION}" in
    preflight|all)
        if [[ $# -gt 0 && "$1" != -* ]]; then SUITE="$1"; shift; fi
        if [[ "${ACTION}" == all ]]; then SUITE="${SUITE:-review-confirmation}"; fi
        ;;
    test) require_value test "${1:-}"; SUITE="$1"; shift ;;
esac
if [[ -n "${SUITE}" ]]; then
    case "${SUITE}" in
        review-submit|review-confirmation|audio-conversion|task-events|task-callback|transcription-caller|moderation-caller) ;;
        *) printf 'Unknown suite: %s\n' "${SUITE}" >&2; exit 2 ;;
    esac
fi

deploy_options=()
test_options=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --region|--project-name|--environment|--tenant-id|--database-parameter-name|--turnstile-secret-parameter-name)
            require_value "$@"; deploy_options+=("$1" "$2"); test_options+=("$1" "$2"); shift 2 ;;
        --audio-file)
            require_value "$@"; test_options+=("$1" "$2"); shift 2 ;;
        --turnstile-allowed-hostnames|--modal-token-id-parameter-name|--modal-token-secret-parameter-name|--transcription-endpoint-url|--modal-endpoint-url|--modal-workspace-id|--cloudflare-zone-name|--public-api-domain-name|--task-events-domain-name|--image-tag)
            require_value "$@"; deploy_options+=("$1" "$2"); shift 2 ;;
        --enable-cloudflare-proxy|--auto-approve) deploy_options+=("$1"); shift ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'Unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
done

case "${ACTION}" in
    preflight)
        if [[ -n "${SUITE}" ]]; then
            exec "${ROOT_DIR}/scripts/test-deployed.sh" preflight "${SUITE}" "${test_options[@]}"
        else
            exec "${ROOT_DIR}/scripts/deploy.sh" preflight "${deploy_options[@]}"
        fi
        ;;
    deploy) exec "${ROOT_DIR}/scripts/deploy.sh" deploy "${deploy_options[@]}" ;;
    test) exec "${ROOT_DIR}/scripts/test-deployed.sh" test "${SUITE}" "${test_options[@]}" ;;
    all)
        # A first deployment does not yet have a WebSocket endpoint. Deferral
        # applies only to this initial check; the actual test validates it again.
        AWS_PROFILE=admin TEST_ALLOW_MISSING_DEPLOYED_ENDPOINT=true "${ROOT_DIR}/scripts/test-deployed.sh" preflight "${SUITE}" "${test_options[@]}"
        AWS_PROFILE=admin "${ROOT_DIR}/scripts/deploy.sh" deploy "${deploy_options[@]}"
        AWS_PROFILE=admin "${ROOT_DIR}/scripts/test-deployed.sh" test "${SUITE}" "${test_options[@]}"
        ;;
esac
