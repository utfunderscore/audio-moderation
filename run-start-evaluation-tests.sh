#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

# Use the deployment profile for Terraform and AWS CLI discovery as well as the
# SDK calls made by the deployed tests.
export AWS_PROFILE=admin

usage() {
    cat <<'EOF'
Usage: ./run-start-evaluation-tests.sh [--audio-file FILE]

Build and run the six ignored deployed start-evaluation integration tests.

Configuration is resolved automatically when omitted:
  AUDIO_MODERATION_API_ENDPOINT     Terraform api_endpoint output
  AWS_REGION                        Region in Terraform's audio-processing state machine ARN
  AUDIO_MODERATION_TENANT_ID        TENANT_ID in the deployed start-evaluation Lambda
  DATABASE_URL                      Decrypted SSM parameter named by that Lambda

Optional explicit overrides:
  AUDIO_MODERATION_API_ENDPOINT, AWS_REGION, AUDIO_MODERATION_TENANT_ID,
  and DATABASE_URL. Explicit values are never replaced.

Audio fixtures (choose one):
  AUDIO_MODERATION_TEST_AUDIO_S3_URIS
                                      JSON array of pre-uploaded audio S3 URIs
  --audio-file FILE                   Upload FILE twice to unique S3 object keys
                                      and use those object URIs for this run

Automatic configuration discovery requires applied Terraform, AWS credentials
for the local admin profile, and (as needed) terraform, aws, and jq. It reads
the Lambda configuration and decrypts its database SSM parameter without
printing the database URL. The script does not deploy infrastructure or
provision fixtures unless --audio-file is provided. Uploaded fixture objects
are retained because the workflows are asynchronous and the bucket lifecycle
manages their eventual removal.
EOF
}

is_blank() {
    [[ -z "${1//[[:space:]]/}" ]]
}

require_value() {
    if [[ $# -lt 2 ]] || is_blank "$2" || [[ "$2" == -* ]]; then
        printf 'Missing value for %s\n' "$1" >&2
        usage >&2
        exit 2
    fi
}

audio_file=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --audio-file)
            require_value "$@"
            audio_file="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            printf 'Unknown option: %s\n' "$1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [[ -n "${audio_file}" ]]; then
    if [[ "${audio_file}" != /* ]]; then
        audio_file="$(pwd -P)/${audio_file}"
    fi

    if [[ ! -f "${audio_file}" || ! -r "${audio_file}" || ! -s "${audio_file}" ]]; then
        printf 'Audio file must be a regular, readable, nonempty file: %s\n' "${audio_file}" >&2
        exit 1
    fi

    if ! is_blank "${AUDIO_MODERATION_TEST_AUDIO_S3_URIS:-}"; then
        printf 'Specify either --audio-file or AUDIO_MODERATION_TEST_AUDIO_S3_URIS, not both.\n' >&2
        exit 2
    fi
fi

if ! command -v cargo >/dev/null 2>&1; then
    printf 'Required command not found: cargo\n' >&2
    exit 1
fi

require_command() {
    local command="$1"

    if ! command -v "${command}" >/dev/null 2>&1; then
        printf 'Required command not found: %s\n' "${command}" >&2
        exit 1
    fi
}

if is_blank "${AUDIO_MODERATION_API_ENDPOINT:-}"; then
    require_command terraform
    AUDIO_MODERATION_API_ENDPOINT="$(terraform -chdir="${ROOT_DIR}/terraform" output -raw api_endpoint)"
    export AUDIO_MODERATION_API_ENDPOINT
fi

if is_blank "${AWS_REGION:-}"; then
    require_command terraform
    state_machine_arn="$(terraform -chdir="${ROOT_DIR}/terraform" output -raw audio_processing_state_machine_arn)"
    IFS=':' read -r _ _ _ discovered_region _ <<<"${state_machine_arn}"
    if is_blank "${discovered_region:-}"; then
        printf 'Could not derive AWS_REGION from Terraform audio_processing_state_machine_arn output.\n' >&2
        exit 1
    fi
    AWS_REGION="${discovered_region}"
    export AWS_REGION
fi

if is_blank "${AUDIO_MODERATION_TENANT_ID:-}" || is_blank "${DATABASE_URL:-}"; then
    require_command terraform
    require_command aws
    require_command jq

    start_evaluation_function_name="$(terraform -chdir="${ROOT_DIR}/terraform" output -raw start_evaluation_function_name)"
    lambda_environment="$(aws lambda get-function-configuration \
        --region "${AWS_REGION}" \
        --function-name "${start_evaluation_function_name}" \
        --query 'Environment.Variables' \
        --output json)"
    lambda_tenant_id="$(jq -r '.TENANT_ID // empty' <<<"${lambda_environment}")"
    database_url_parameter="$(jq -r '.DATABASE_URL_PARAMETER // empty' <<<"${lambda_environment}")"

    if is_blank "${AUDIO_MODERATION_TENANT_ID:-}"; then
        AUDIO_MODERATION_TENANT_ID="${lambda_tenant_id}"
        export AUDIO_MODERATION_TENANT_ID
    fi

    if is_blank "${DATABASE_URL:-}"; then
        if is_blank "${database_url_parameter}"; then
            printf 'Could not resolve DATABASE_URL: deployed Lambda has no DATABASE_URL_PARAMETER.\n' >&2
            exit 1
        fi
        DATABASE_URL="$(aws ssm get-parameter \
            --region "${AWS_REGION}" \
            --name "${database_url_parameter}" \
            --with-decryption \
            --query 'Parameter.Value' \
            --output text)"
        export DATABASE_URL
    fi
fi

if [[ -n "${audio_file}" ]]; then
    # These are needed for fixture provisioning even when all other test
    # configuration was supplied explicitly.
    require_command terraform
    require_command aws
    require_command jq

    uploads_bucket_name="$(terraform -chdir="${ROOT_DIR}/terraform" output -raw uploads_bucket_name)"
    if is_blank "${uploads_bucket_name}"; then
        printf 'Could not resolve uploads_bucket_name from Terraform output.\n' >&2
        exit 1
    fi

    if [[ ! -r /proc/sys/kernel/random/uuid ]]; then
        printf 'Could not generate a unique fixture run ID: /proc/sys/kernel/random/uuid is unavailable.\n' >&2
        exit 1
    fi
    IFS= read -r fixture_run_id </proc/sys/kernel/random/uuid
    if is_blank "${fixture_run_id}"; then
        printf 'Could not generate a unique fixture run ID.\n' >&2
        exit 1
    fi

    first_audio_s3_uri="s3://${uploads_bucket_name}/reviews/${fixture_run_id}/first/source"
    second_audio_s3_uri="s3://${uploads_bucket_name}/reviews/${fixture_run_id}/second/source"
    aws s3 cp "${audio_file}" "${first_audio_s3_uri}" --region "${AWS_REGION}"
    aws s3 cp "${audio_file}" "${second_audio_s3_uri}" --region "${AWS_REGION}"
    AUDIO_MODERATION_TEST_AUDIO_S3_URIS="$(jq -cn \
        --arg first "${first_audio_s3_uri}" \
        --arg second "${second_audio_s3_uri}" \
        '[$first, $second]')"
    export AUDIO_MODERATION_TEST_AUDIO_S3_URIS
elif is_blank "${AUDIO_MODERATION_TEST_AUDIO_S3_URIS:-}"; then
    printf 'Provide --audio-file FILE or set AUDIO_MODERATION_TEST_AUDIO_S3_URIS to a JSON array of at least two pre-uploaded, valid audio S3 URIs.\n' >&2
    exit 1
fi

for variable in AUDIO_MODERATION_API_ENDPOINT DATABASE_URL AUDIO_MODERATION_TENANT_ID; do
    value="${!variable:-}"
    if is_blank "${value}"; then
        printf 'Required configuration could not be resolved: %s. Set it explicitly or ensure Terraform and the deployed Lambda are available.\n' "${variable}" >&2
        exit 1
    fi
done

# Running from the workspace root ensures Cargo reads .cargo/config.toml,
# including the SQLx offline configuration, regardless of the caller's cwd.
cd "${ROOT_DIR}"

cargo test --package start-evaluation-lambda --test deployed --no-run
cargo test --package start-evaluation-lambda --test deployed -- --ignored --nocapture
