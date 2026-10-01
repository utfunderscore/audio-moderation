#!/usr/bin/env bash

validate_turnstile_configuration() {
    if [[ ! "${TURNSTILE_SECRET_KEY_PARAMETER}" =~ ^/[a-zA-Z0-9_.-]+(/[a-zA-Z0-9_.-]+)*$ ]]; then
        printf 'Turnstile secret parameter name must be an absolute SSM path.\n' >&2
        exit 1
    fi
    if [[ -z "${TURNSTILE_ALLOWED_HOSTNAMES}" ]]; then
        printf 'Set --turnstile-allowed-hostnames to the exact frontend hostnames before deploying.\n' >&2
        exit 1
    fi
    local hostname
    local -a hostnames
    IFS=',' read -r -a hostnames <<< "${TURNSTILE_ALLOWED_HOSTNAMES}"
    if [[ "${TURNSTILE_ALLOWED_HOSTNAMES}" == *, ]]; then
        printf 'Turnstile allowed hostnames cannot contain an empty entry.\n' >&2
        exit 1
    fi
    for hostname in "${hostnames[@]}"; do
        if [[ ! "${hostname}" =~ ^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)*$ ]]; then
            printf 'Invalid Turnstile hostname: %s (use exact lowercase hostnames without schemes, ports, paths, whitespace, or wildcards).\n' "${hostname}" >&2
            exit 1
        fi
    done
}

validate_modal_oidc_provider() {
    local provider_arn="arn:aws:iam::${ACCOUNT_ID}:oidc-provider/oidc.modal.com" provider_url
    provider_url="$(AWS_PROFILE=admin aws iam get-open-id-connect-provider --open-id-connect-provider-arn "${provider_arn}" --query 'Url' --output text)"
    if [[ "${provider_url}" != "oidc.modal.com" ]]; then
        printf 'Modal OIDC provider is missing or unexpected: %s\n' "${provider_arn}" >&2
        exit 1
    fi
    printf 'Validated Modal OIDC provider: %s\n' "${provider_arn}"
}

validate_transcription_endpoint() {
    if [[ -z "${TRANSCRIPTION_ENDPOINT_URL}" ]]; then
        TRANSCRIPTION_ENDPOINT_URL="$(deployment_output transcription_endpoint_url 2>/dev/null || true)"
    fi
    if [[ -z "${TRANSCRIPTION_ENDPOINT_URL}" ]]; then
        TRANSCRIPTION_ENDPOINT_URL="$(AWS_PROFILE=admin aws lambda get-function-configuration --region "${AWS_REGION}" --function-name "${PROJECT_NAME}-${ENVIRONMENT}-transcription-caller" --query 'Environment.Variables.TRANSCRIPTION_ENDPOINT_URL' --output text 2>/dev/null || true)"
        if [[ "${TRANSCRIPTION_ENDPOINT_URL}" == "None" ]]; then TRANSCRIPTION_ENDPOINT_URL=""; fi
    fi
    if [[ -z "${TRANSCRIPTION_ENDPOINT_URL}" || ! "${TRANSCRIPTION_ENDPOINT_URL}" =~ ^https:// ]]; then
        printf 'An HTTPS transcription endpoint is required; pass --transcription-endpoint-url.\n' >&2
        exit 1
    fi
    printf 'Validated configured transcription endpoint without contacting it.\n'
}

validate_modal_endpoint() {
    if [[ -z "${MODAL_ENDPOINT_URL}" ]]; then
        MODAL_ENDPOINT_URL="$(deployment_output modal_endpoint_url 2>/dev/null || true)"
    fi
    if [[ -z "${MODAL_ENDPOINT_URL}" ]]; then
        MODAL_ENDPOINT_URL="$(AWS_PROFILE=admin aws lambda get-function-configuration --region "${AWS_REGION}" --function-name "${PROJECT_NAME}-${ENVIRONMENT}-moderation-caller" --query 'Environment.Variables.MODAL_ENDPOINT_URL' --output text 2>/dev/null || true)"
        if [[ "${MODAL_ENDPOINT_URL}" == "None" ]]; then MODAL_ENDPOINT_URL=""; fi
    fi
    if [[ -z "${MODAL_ENDPOINT_URL}" || ! "${MODAL_ENDPOINT_URL}" =~ ^https:// ]]; then
        printf 'An HTTPS Modal endpoint is required; pass --modal-endpoint-url.\n' >&2
        exit 1
    fi
    printf 'Validated configured Modal endpoint without contacting it.\n'
}

deployment_preflight() {
    validate_turnstile_configuration
    require_command terraform
    validate_aws_context
    printf 'Terraform uses local state in terraform/. Do not run deploys concurrently from separate worktrees; local state has no shared lock.\n'
    if [[ -z "${CLOUDFLARE_API_TOKEN:-}" && ( -z "${CLOUDFLARE_API_KEY:-}" || -z "${CLOUDFLARE_EMAIL:-}" ) ]]; then
        printf 'Cloudflare credentials are required to create the Turnstile widget: set CLOUDFLARE_API_TOKEN, or both CLOUDFLARE_API_KEY and CLOUDFLARE_EMAIL.\n' >&2
        exit 1
    fi
    printf 'Cloudflare Turnstile widget account is resolved from zone %s.\n' "${CLOUDFLARE_ZONE_NAME}"
    if [[ "${ENABLE_CLOUDFLARE_PROXY}" == true ]]; then
        printf 'Cloudflare proxy target: https://%s and wss://%s (zone %s).\n' \
            "${PUBLIC_API_DOMAIN_NAME}" "${TASK_EVENTS_DOMAIN_NAME}" "${CLOUDFLARE_ZONE_NAME}"
    fi
    local command
    for command in cargo docker git jq; do require_command "${command}"; done
    validate_secure_parameter "${DATABASE_URL_PARAMETER}"
    validate_secure_parameter "${MODAL_PROXY_TOKEN_ID_PARAMETER}"
    validate_secure_parameter "${MODAL_PROXY_TOKEN_SECRET_PARAMETER}"
    validate_modal_oidc_provider
    validate_transcription_endpoint
    validate_modal_endpoint
    printf 'Database schema prerequisite: the current review-job, pipeline-task, task-event, and WebSocket schemas must be applied before deployment. This runner deliberately does not run migrations.\n'
}
