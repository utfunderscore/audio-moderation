#!/usr/bin/env bash

TENANT_ID="${TENANT_ID:-default}"
TURNSTILE_SECRET_KEY_PARAMETER="${TURNSTILE_SECRET_KEY_PARAMETER:-}"
TURNSTILE_ALLOWED_HOSTNAMES="${TURNSTILE_ALLOWED_HOSTNAMES:-}"
MODAL_PROXY_TOKEN_ID_PARAMETER="${MODAL_PROXY_TOKEN_ID_PARAMETER:-}"
MODAL_PROXY_TOKEN_SECRET_PARAMETER="${MODAL_PROXY_TOKEN_SECRET_PARAMETER:-}"
TRANSCRIPTION_ENDPOINT_URL="${TRANSCRIPTION_ENDPOINT_URL:-}"
MODAL_ENDPOINT_URL="${MODAL_ENDPOINT_URL:-}"
MODAL_WORKSPACE_ID="${MODAL_WORKSPACE_ID:-ac-k4lbrkEynY351mickkxfRh}"
IMAGE_TAG="${IMAGE_TAG:-}"
ENABLE_CLOUDFLARE_PROXY="${ENABLE_CLOUDFLARE_PROXY:-false}"
CLOUDFLARE_ZONE_NAME="${CLOUDFLARE_ZONE_NAME:-utf.lol}"
PUBLIC_API_DOMAIN_NAME="${PUBLIC_API_DOMAIN_NAME:-api-guard.utf.lol}"
TASK_EVENTS_DOMAIN_NAME="${TASK_EVENTS_DOMAIN_NAME:-events-guard.utf.lol}"
AUTO_APPROVE=false

parse_deployment_options() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --region) require_value "$@"; AWS_REGION="$2"; shift 2 ;;
            --project-name) require_value "$@"; PROJECT_NAME="$2"; shift 2 ;;
            --environment) require_value "$@"; ENVIRONMENT="$2"; shift 2 ;;
            --tenant-id) require_value "$@"; TENANT_ID="$2"; shift 2 ;;
            --database-parameter-name) require_value "$@"; DATABASE_URL_PARAMETER="$2"; shift 2 ;;
            --turnstile-secret-parameter-name) require_value "$@"; TURNSTILE_SECRET_KEY_PARAMETER="$2"; shift 2 ;;
            --turnstile-allowed-hostnames) require_value "$@"; TURNSTILE_ALLOWED_HOSTNAMES="$2"; shift 2 ;;
            --modal-token-id-parameter-name) require_value "$@"; MODAL_PROXY_TOKEN_ID_PARAMETER="$2"; shift 2 ;;
            --modal-token-secret-parameter-name) require_value "$@"; MODAL_PROXY_TOKEN_SECRET_PARAMETER="$2"; shift 2 ;;
            --transcription-endpoint-url) require_value "$@"; TRANSCRIPTION_ENDPOINT_URL="$2"; shift 2 ;;
            --modal-endpoint-url) require_value "$@"; MODAL_ENDPOINT_URL="$2"; shift 2 ;;
            --modal-workspace-id) require_value "$@"; MODAL_WORKSPACE_ID="$2"; shift 2 ;;
            --enable-cloudflare-proxy) ENABLE_CLOUDFLARE_PROXY=true; shift ;;
            --cloudflare-zone-name) require_value "$@"; CLOUDFLARE_ZONE_NAME="$2"; shift 2 ;;
            --public-api-domain-name) require_value "$@"; PUBLIC_API_DOMAIN_NAME="$2"; shift 2 ;;
            --task-events-domain-name) require_value "$@"; TASK_EVENTS_DOMAIN_NAME="$2"; shift 2 ;;
            --image-tag) require_value "$@"; IMAGE_TAG="$2"; shift 2 ;;
            --auto-approve) AUTO_APPROVE=true; shift ;;
            -h|--help) usage; exit 0 ;;
            *) printf 'Unknown deployment option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
        esac
    done
}

configure_deployment() {
    TURNSTILE_SECRET_KEY_PARAMETER="${TURNSTILE_SECRET_KEY_PARAMETER:-/${PROJECT_NAME}/${ENVIRONMENT}/turnstile-secret-key}"
    MODAL_PROXY_TOKEN_ID_PARAMETER="${MODAL_PROXY_TOKEN_ID_PARAMETER:-/${PROJECT_NAME}/${ENVIRONMENT}/modal-proxy-token-id}"
    MODAL_PROXY_TOKEN_SECRET_PARAMETER="${MODAL_PROXY_TOKEN_SECRET_PARAMETER:-/${PROJECT_NAME}/${ENVIRONMENT}/modal-proxy-token-secret}"
}
