#!/usr/bin/env bash

TENANT_ID="${TENANT_ID:-default}"
TURNSTILE_SECRET_KEY_PARAMETER="${TURNSTILE_SECRET_KEY_PARAMETER:-}"
AUDIO_FILE="${AUDIO_FILE:-}"
TASK_EVENTS_ENDPOINT="${AUDIO_MODERATION_TASK_EVENTS_ENDPOINT:-}"

parse_test_options() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --region) require_value "$@"; AWS_REGION="$2"; shift 2 ;;
            --project-name) require_value "$@"; PROJECT_NAME="$2"; shift 2 ;;
            --environment) require_value "$@"; ENVIRONMENT="$2"; shift 2 ;;
            --tenant-id) require_value "$@"; TENANT_ID="$2"; shift 2 ;;
            --database-parameter-name) require_value "$@"; DATABASE_URL_PARAMETER="$2"; shift 2 ;;
            --turnstile-secret-parameter-name) require_value "$@"; TURNSTILE_SECRET_KEY_PARAMETER="$2"; shift 2 ;;
            --audio-file) require_value "$@"; AUDIO_FILE="$2"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *) printf 'Unknown deployed-test option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
        esac
    done
}

configure_test_environment() {
    TURNSTILE_SECRET_KEY_PARAMETER="${TURNSTILE_SECRET_KEY_PARAMETER:-/${PROJECT_NAME}/${ENVIRONMENT}/turnstile-secret-key}"
    if [[ -n "${AUDIO_FILE}" && "${AUDIO_FILE}" != /* ]]; then
        AUDIO_FILE="$(pwd -P)/${AUDIO_FILE}"
    fi
}

load_api_endpoint() {
    if [[ -z "${AUDIO_MODERATION_API_ENDPOINT:-}" ]]; then
        AUDIO_MODERATION_API_ENDPOINT="$(deployment_output api_endpoint)"
        export AUDIO_MODERATION_API_ENDPOINT
    fi
}

load_pipeline_database_environment() {
    AUDIO_MODERATION_TENANT_ID="${AUDIO_MODERATION_TENANT_ID:-$(deployment_output tenant_id)}"
    export AUDIO_MODERATION_TENANT_ID
    if [[ -z "${DATABASE_URL:-}" ]]; then
        DATABASE_URL="$(AWS_PROFILE=admin aws ssm get-parameter --region "${AWS_REGION}" --name "${DATABASE_URL_PARAMETER}" --with-decryption --query 'Parameter.Value' --output text)"
        export DATABASE_URL
    fi
}

load_task_events_endpoint() {
    validate_task_events_endpoint false
    AUDIO_MODERATION_TASK_EVENTS_ENDPOINT="${TASK_EVENTS_ENDPOINT}"
    export AUDIO_MODERATION_TASK_EVENTS_ENDPOINT
}

run_deployed_cargo_test() {
    # Running inside backend/ loads its .cargo/config.toml (SQLX_OFFLINE=true).
    (cd "${BACKEND_DIR}" && cargo test --package "$1" --test deployed "$2" -- --ignored --nocapture)
}
