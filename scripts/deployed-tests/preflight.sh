#!/usr/bin/env bash

validate_turnstile_test_token() {
    if [[ -z "${TURNSTILE_TEST_TOKEN:-}" ]]; then
        printf 'TURNSTILE_TEST_TOKEN is required for deployed review tests (do not print or commit the token).\n' >&2
        exit 1
    fi
    if [[ ! "${TURNSTILE_SECRET_KEY_PARAMETER}" =~ ^/[a-zA-Z0-9_.-]+(/[a-zA-Z0-9_.-]+)*$ ]]; then
        printf 'Turnstile secret parameter name must be an absolute SSM path.\n' >&2
        exit 1
    fi
}

validate_audio_file() {
    if [[ -z "${AUDIO_FILE}" || ! -f "${AUDIO_FILE}" || ! -r "${AUDIO_FILE}" || ! -s "${AUDIO_FILE}" ]]; then
        printf 'A regular, readable, nonempty --audio-file is required for %s.\n' "${SUITE}" >&2
        exit 1
    fi
}

validate_task_events_endpoint() {
    local allow_missing_deployed_endpoint="${1:-false}"
    if [[ -z "${TASK_EVENTS_ENDPOINT}" ]]; then
        TASK_EVENTS_ENDPOINT="$(deployment_output pipeline_task_events_websocket_endpoint 2>/dev/null || true)"
    fi
    if [[ -z "${TASK_EVENTS_ENDPOINT}" && "${allow_missing_deployed_endpoint}" == true ]]; then
        printf 'Task-events WebSocket endpoint is not expected until this deployment completes; deferring deployed endpoint validation.\n'
        return
    fi
    if [[ -z "${TASK_EVENTS_ENDPOINT}" || ! "${TASK_EVENTS_ENDPOINT}" =~ ^wss://[^[:space:]]+$ ]]; then
        printf 'A deployed task-events wss:// endpoint is required for %s. Deploy first or set AUDIO_MODERATION_TASK_EVENTS_ENDPOINT.\n' "${SUITE}" >&2
        exit 1
    fi
    printf 'Validated deployed task-events WebSocket endpoint.\n'
}

test_preflight() {
    local allow_missing_deployed_endpoint="${1:-false}"
    # Fail local prerequisites before any AWS request.
    case "${SUITE}" in
        review-submit|review-confirmation) validate_turnstile_test_token ;;
        audio-conversion) validate_audio_file ;;
    esac
    require_command terraform
    validate_aws_context
    suite_preflight "${allow_missing_deployed_endpoint}"
}
