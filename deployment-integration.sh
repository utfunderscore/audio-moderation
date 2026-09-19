#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
TERRAFORM_DIR="${ROOT_DIR}/terraform"
BACKEND_DIR="${ROOT_DIR}/backend"

# Deployment and deployed tests must never accidentally select a developer's
# default AWS profile.
export AWS_PROFILE=admin

AWS_REGION="${AWS_REGION:-eu-west-2}"
PROJECT_NAME="${PROJECT_NAME:-audio-moderation}"
ENVIRONMENT="${ENVIRONMENT:-dev}"
TENANT_ID="${TENANT_ID:-default}"
DATABASE_URL_PARAMETER="${DATABASE_URL_PARAMETER:-}"
MODAL_PROXY_TOKEN_ID_PARAMETER="${MODAL_PROXY_TOKEN_ID_PARAMETER:-}"
MODAL_PROXY_TOKEN_SECRET_PARAMETER="${MODAL_PROXY_TOKEN_SECRET_PARAMETER:-}"
TRANSCRIPTION_ENDPOINT_URL="${TRANSCRIPTION_ENDPOINT_URL:-}"
MODAL_ENDPOINT_URL="${MODAL_ENDPOINT_URL:-}"
MODAL_WORKSPACE_ID="${MODAL_WORKSPACE_ID:-ac-k4lbrkEynY351mickkxfRh}"
IMAGE_TAG="${IMAGE_TAG:-}"
AUDIO_FILE="${AUDIO_FILE:-}"
TASK_EVENTS_ENDPOINT="${AUDIO_MODERATION_TASK_EVENTS_ENDPOINT:-}"
AUTO_APPROVE=false

usage() {
    cat <<'EOF'
Usage:
  AWS_PROFILE=admin ./deployment-integration.sh preflight [suite] [options]
  AWS_PROFILE=admin ./deployment-integration.sh deploy [options]
  AWS_PROFILE=admin ./deployment-integration.sh test <suite> [options]
  AWS_PROFILE=admin ./deployment-integration.sh all [suite] [options]

Commands:
  preflight [suite]  Validate prerequisites. Without a suite, validate a full deployment.
  deploy             Bootstrap eight ECR repositories, build and push eight images with one
                     immutable tag, then perform one full Terraform apply.
  test <suite>       Run one deployed suite without changing deployed infrastructure.
  all [suite]        Deploy, then run a suite (default: evaluation-e2e).

Suites, ordered from isolated/cheap to full:
  review-submit          SubmitReview creates an awaiting-upload review.
  review-confirmation    A presigned source upload triggers confirm-upload.
  evaluation-ingress     StartEvaluation validation, lease/terminal retry behavior, and
                         idempotency conflicts, with synthetic non-dispatched audio URIs.
  evaluation-dispatch    Seeded dispatch and retry through the production workflow. Requires
                         an audio fixture; executions continue asynchronously.
  audio-conversion       Direct synchronous audio-processing Lambda invocation.
  task-events            WebSocket task-event replay, live delivery, and disconnect cleanup.
  task-callback          Unsupported: requires an ASR-created persisted callback attempt.
  transcription-caller   Unsupported: requires a compatible external transcription API and
                          a safe task-token/task fixture harness that does not yet exist.
  moderation-caller      Unsupported: requires completed transcription and task-token fixtures.
  evaluation-e2e         StartEvaluation through conversion, transcription, moderation,
                          callbacks, and terminal workflow completion.

Options:
  --region REGION                         AWS region (default: eu-west-2)
  --project-name NAME                     Project name (default: audio-moderation)
  --environment NAME                      Environment (default: dev)
  --tenant-id ID                          Tenant ID (default: default)
  --database-parameter-name NAME          SecureString database URL parameter
  --modal-token-id-parameter-name NAME    SecureString Modal token ID parameter
  --modal-token-secret-parameter-name NAME
                                            SecureString Modal token secret parameter
  --transcription-endpoint-url URL        External transcription endpoint
                                            (default: the Terraform transcription_endpoint_url
                                            variable / deployed transcription-caller)
  --modal-workspace-id ID                 Modal workspace ID
  --modal-endpoint-url URL                Base URL for the Modal moderation API
  --audio-file FILE                       Readable audio fixture for audio-conversion or e2e
  --image-tag TAG                         Immutable image tag (deploy only; default is git SHA
                                            plus UTC timestamp)
  --auto-approve                          Skip Terraform approval prompts (deploy/all only)
  -h, --help                              Show this help

Configuration can also be supplied through the corresponding upper-case
environment variables. The script always uses AWS_PROFILE=admin and never
prints decrypted parameter values.
EOF
}

require_value() {
    if [[ $# -lt 2 ]] || [[ -z "$2" || "$2" == -* ]]; then
        printf 'Missing value for %s\n' "$1" >&2
        usage >&2
        exit 2
    fi
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'Required command not found: %s\n' "$1" >&2
        exit 1
    fi
}

is_suite() {
    case "$1" in
        review-submit|review-confirmation|evaluation-ingress|evaluation-dispatch|audio-conversion|task-events|task-callback|transcription-caller|moderation-caller|evaluation-e2e) return 0 ;;
        *) return 1 ;;
    esac
}

ACTION="${1:-}"
if [[ -z "${ACTION}" || "${ACTION}" == -h || "${ACTION}" == --help ]]; then
    usage
    exit 0
fi
shift

SUITE=""
case "${ACTION}" in
    preflight)
        if [[ $# -gt 0 && "$1" != -* ]]; then
            SUITE="$1"
            shift
        fi
        ;;
    deploy)
        ;;
    test)
        require_value test "${1:-}"
        SUITE="$1"
        shift
        ;;
    all)
        if [[ $# -gt 0 && "$1" != -* ]]; then
            SUITE="$1"
            shift
        else
            SUITE="evaluation-e2e"
        fi
        ;;
    *)
        printf 'Unknown command: %s\n' "${ACTION}" >&2
        usage >&2
        exit 2
        ;;
esac

if [[ -n "${SUITE}" ]] && ! is_suite "${SUITE}"; then
    printf 'Unknown suite: %s\n' "${SUITE}" >&2
    usage >&2
    exit 2
fi

while [[ $# -gt 0 ]]; do
    case "$1" in
        --region) require_value "$@"; AWS_REGION="$2"; shift 2 ;;
        --project-name) require_value "$@"; PROJECT_NAME="$2"; shift 2 ;;
        --environment) require_value "$@"; ENVIRONMENT="$2"; shift 2 ;;
        --tenant-id) require_value "$@"; TENANT_ID="$2"; shift 2 ;;
        --database-parameter-name) require_value "$@"; DATABASE_URL_PARAMETER="$2"; shift 2 ;;
        --modal-token-id-parameter-name) require_value "$@"; MODAL_PROXY_TOKEN_ID_PARAMETER="$2"; shift 2 ;;
        --modal-token-secret-parameter-name) require_value "$@"; MODAL_PROXY_TOKEN_SECRET_PARAMETER="$2"; shift 2 ;;
        --transcription-endpoint-url) require_value "$@"; TRANSCRIPTION_ENDPOINT_URL="$2"; shift 2 ;;
        --modal-endpoint-url) require_value "$@"; MODAL_ENDPOINT_URL="$2"; shift 2 ;;
        --modal-workspace-id) require_value "$@"; MODAL_WORKSPACE_ID="$2"; shift 2 ;;
        --audio-file) require_value "$@"; AUDIO_FILE="$2"; shift 2 ;;
        --image-tag) require_value "$@"; IMAGE_TAG="$2"; shift 2 ;;
        --auto-approve) AUTO_APPROVE=true; shift ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'Unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
done

# Rust AWS SDK clients use this environment variable rather than the AWS CLI's
# per-command --region flag.
export AWS_REGION

DATABASE_URL_PARAMETER="${DATABASE_URL_PARAMETER:-/${PROJECT_NAME}/${ENVIRONMENT}/database-url}"
MODAL_PROXY_TOKEN_ID_PARAMETER="${MODAL_PROXY_TOKEN_ID_PARAMETER:-/${PROJECT_NAME}/${ENVIRONMENT}/modal-proxy-token-id}"
MODAL_PROXY_TOKEN_SECRET_PARAMETER="${MODAL_PROXY_TOKEN_SECRET_PARAMETER:-/${PROJECT_NAME}/${ENVIRONMENT}/modal-proxy-token-secret}"

if [[ -n "${AUDIO_FILE}" && "${AUDIO_FILE}" != /* ]]; then
    AUDIO_FILE="$(pwd -P)/${AUDIO_FILE}"
fi

validate_audio_file() {
    if [[ -z "${AUDIO_FILE}" || ! -f "${AUDIO_FILE}" || ! -r "${AUDIO_FILE}" || ! -s "${AUDIO_FILE}" ]]; then
        printf 'A regular, readable, nonempty --audio-file is required for %s.\n' "${SUITE:-this command}" >&2
        exit 1
    fi
}

validate_secure_parameter() {
    local parameter_name="$1"
    local parameter_type
    parameter_type="$(aws ssm get-parameter --region "${AWS_REGION}" --name "${parameter_name}" --query 'Parameter.Type' --output text)"
    if [[ "${parameter_type}" != "SecureString" ]]; then
        printf 'Required encrypted SSM parameter is not a SecureString: %s\n' "${parameter_name}" >&2
        exit 1
    fi
    printf 'Validated encrypted SSM parameter: %s\n' "${parameter_name}"
}

validate_modal_oidc_provider() {
    local provider_arn="arn:aws:iam::${ACCOUNT_ID}:oidc-provider/oidc.modal.com"
    local provider_url
    provider_url="$(aws iam get-open-id-connect-provider --open-id-connect-provider-arn "${provider_arn}" --query 'Url' --output text)"
    if [[ "${provider_url}" != "oidc.modal.com" ]]; then
        printf 'Modal OIDC provider is missing or unexpected: %s\n' "${provider_arn}" >&2
        exit 1
    fi
    printf 'Validated Modal OIDC provider: %s\n' "${provider_arn}"
}

validate_transcription_endpoint() {
    if [[ -z "${TRANSCRIPTION_ENDPOINT_URL}" ]]; then
        TRANSCRIPTION_ENDPOINT_URL="$(terraform -chdir="${TERRAFORM_DIR}" output -raw transcription_endpoint_url 2>/dev/null || true)"
    fi
    if [[ -z "${TRANSCRIPTION_ENDPOINT_URL}" ]]; then
        TRANSCRIPTION_ENDPOINT_URL="$(aws lambda get-function-configuration --region "${AWS_REGION}" --function-name "${PROJECT_NAME}-${ENVIRONMENT}-transcription-caller" --query 'Environment.Variables.TRANSCRIPTION_ENDPOINT_URL' --output text 2>/dev/null || true)"
        if [[ "${TRANSCRIPTION_ENDPOINT_URL}" == "None" ]]; then
            TRANSCRIPTION_ENDPOINT_URL=""
        fi
    fi
    if [[ -z "${TRANSCRIPTION_ENDPOINT_URL}" || ! "${TRANSCRIPTION_ENDPOINT_URL}" =~ ^https:// ]]; then
        printf 'An HTTPS transcription endpoint is required; pass --transcription-endpoint-url.\n' >&2
        exit 1
    fi
    printf 'Validated configured transcription endpoint without contacting it.\n'
}

validate_modal_endpoint() {
    if [[ -z "${MODAL_ENDPOINT_URL}" ]]; then
        MODAL_ENDPOINT_URL="$(terraform -chdir="${TERRAFORM_DIR}" output -raw modal_endpoint_url 2>/dev/null || true)"
    fi
    if [[ -z "${MODAL_ENDPOINT_URL}" ]]; then
        MODAL_ENDPOINT_URL="$(aws lambda get-function-configuration --region "${AWS_REGION}" --function-name "${PROJECT_NAME}-${ENVIRONMENT}-moderation-caller" --query 'Environment.Variables.MODAL_ENDPOINT_URL' --output text 2>/dev/null || true)"
        if [[ "${MODAL_ENDPOINT_URL}" == "None" ]]; then
            MODAL_ENDPOINT_URL=""
        fi
    fi
    if [[ -z "${MODAL_ENDPOINT_URL}" || ! "${MODAL_ENDPOINT_URL}" =~ ^https:// ]]; then
        printf 'An HTTPS Modal endpoint is required; pass --modal-endpoint-url.\n' >&2
        exit 1
    fi
    printf 'Validated configured Modal endpoint without contacting it.\n'
}

validate_task_events_endpoint() {
    local allow_missing_deployed_endpoint="${1:-false}"
    if [[ -z "${TASK_EVENTS_ENDPOINT}" ]]; then
        TASK_EVENTS_ENDPOINT="$(terraform -chdir="${TERRAFORM_DIR}" output -raw pipeline_task_events_websocket_endpoint 2>/dev/null || true)"
    fi
    if [[ -z "${TASK_EVENTS_ENDPOINT}" && "${allow_missing_deployed_endpoint}" == true ]]; then
        printf 'Task-events WebSocket endpoint is not expected until this deployment completes; deferring deployed endpoint validation.\n'
        return
    fi
    if [[ -z "${TASK_EVENTS_ENDPOINT}" || ! "${TASK_EVENTS_ENDPOINT}" =~ ^wss://[^[:space:]]+$ ]]; then
        printf 'A deployed task-events wss:// endpoint is required for %s. Deploy first or set AUDIO_MODERATION_TASK_EVENTS_ENDPOINT.\n' "${SUITE:-this command}" >&2
        exit 1
    fi
    printf 'Validated deployed task-events WebSocket endpoint.\n'
}

preflight() {
    local target_suite="${1:-full}"
    local allow_missing_deployed_endpoint="${2:-false}"
    require_command aws
    require_command terraform
    ACCOUNT_ID="$(aws sts get-caller-identity --region "${AWS_REGION}" --query Account --output text)"
    if [[ -z "${ACCOUNT_ID}" || "${ACCOUNT_ID}" == "None" ]]; then
        printf 'Unable to determine the AWS account for AWS_PROFILE=admin.\n' >&2
        exit 1
    fi
    printf 'AWS_PROFILE=admin account: %s\n' "${ACCOUNT_ID}"
    printf 'AWS target region: %s\n' "${AWS_REGION}"
    printf 'Terraform uses local state in terraform/. Do not run deploys concurrently from separate worktrees; local state has no shared lock.\n'

    case "${target_suite}" in
        full)
            for command in cargo docker git jq; do require_command "${command}"; done
            validate_secure_parameter "${DATABASE_URL_PARAMETER}"
            validate_secure_parameter "${MODAL_PROXY_TOKEN_ID_PARAMETER}"
            validate_secure_parameter "${MODAL_PROXY_TOKEN_SECRET_PARAMETER}"
            validate_modal_oidc_provider
            validate_transcription_endpoint
            validate_modal_endpoint
            printf 'Database schema prerequisite: apply the current schema before deployment or deployed tests. This runner deliberately does not run migrations.\n'
            if [[ -n "${AUDIO_FILE}" ]]; then validate_audio_file; fi
            ;;
        review-submit|review-confirmation)
            require_command cargo
            validate_secure_parameter "${DATABASE_URL_PARAMETER}"
            printf 'Database schema prerequisite: the review-job schema must already be current.\n'
            ;;
        evaluation-ingress)
            for command in cargo jq; do require_command "${command}"; done
            validate_secure_parameter "${DATABASE_URL_PARAMETER}"
            printf 'Database schema prerequisite: the pipeline-task schema must already be current.\n'
            ;;
        audio-conversion)
            require_command jq
            validate_audio_file
            ;;
        task-events)
            require_command cargo
            validate_secure_parameter "${DATABASE_URL_PARAMETER}"
            validate_task_events_endpoint "${allow_missing_deployed_endpoint}"
            printf 'Database schema prerequisite: the pipeline-task event and WebSocket connection schemas must already be current.\n'
            ;;
        task-callback)
            ;;
        transcription-caller)
            printf 'transcription-caller is unsupported: it needs a compatible external transcription endpoint plus a database task and a real Step Functions task-token fixture. The checked-in socialguard-models application contract must not be changed to provide that harness.\n' >&2
            exit 1
            ;;
        moderation-caller)
            printf 'moderation-caller is unsupported: it needs a completed transcription plus a real Step Functions task-token fixture.\n' >&2
            exit 1
            ;;
        evaluation-e2e)
            for command in cargo jq; do require_command "${command}"; done
            validate_secure_parameter "${DATABASE_URL_PARAMETER}"
            validate_secure_parameter "${MODAL_PROXY_TOKEN_ID_PARAMETER}"
            validate_secure_parameter "${MODAL_PROXY_TOKEN_SECRET_PARAMETER}"
            validate_modal_oidc_provider
            validate_transcription_endpoint
            validate_modal_endpoint
            validate_task_events_endpoint "${allow_missing_deployed_endpoint}"
            printf 'Database schema prerequisite: the pipeline-task schema must already be current.\n'
            if [[ "${target_suite}" == "evaluation-e2e" ]]; then validate_audio_file; fi
            ;;
        evaluation-dispatch)
            for command in cargo jq; do require_command "${command}"; done
            validate_secure_parameter "${DATABASE_URL_PARAMETER}"
            validate_audio_file
            printf 'Database schema prerequisite: the pipeline-task schema must already be current.\n'
            ;;
    esac
}

terraform_vars=()
set_terraform_vars() {
    terraform_vars=(
        -var="aws_region=${AWS_REGION}"
        -var="project_name=${PROJECT_NAME}"
        -var="environment=${ENVIRONMENT}"
        -var="tenant_id=${TENANT_ID}"
        -var="database_parameter_name=${DATABASE_URL_PARAMETER}"
        -var="modal_proxy_token_id_parameter_name=${MODAL_PROXY_TOKEN_ID_PARAMETER}"
        -var="modal_proxy_token_secret_parameter_name=${MODAL_PROXY_TOKEN_SECRET_PARAMETER}"
        -var="transcription_endpoint_url=${TRANSCRIPTION_ENDPOINT_URL}"
        -var="modal_endpoint_url=${MODAL_ENDPOINT_URL}"
        -var="modal_workspace_id=${MODAL_WORKSPACE_ID}"
        -var="enable_test_resources=true"
        -var="submit_audio_image_tag=${IMAGE_TAG}"
        -var="confirm_upload_image_tag=${IMAGE_TAG}"
        -var="audio_processing_image_tag=${IMAGE_TAG}"
        -var="start_evaluation_image_tag=${IMAGE_TAG}"
        -var="task_callback_image_tag=${IMAGE_TAG}"
        -var="transcription_caller_image_tag=${IMAGE_TAG}"
        -var="moderation_caller_image_tag=${IMAGE_TAG}"
        -var="task_events_image_tag=${IMAGE_TAG}"
    )
    if [[ -n "${TRANSCRIPTION_ENDPOINT_URL}" ]]; then
        terraform_vars+=(-var="transcription_endpoint_url=${TRANSCRIPTION_ENDPOINT_URL}")
    fi
}

deploy() {
    preflight full
    if [[ -z "${IMAGE_TAG}" ]]; then
        IMAGE_TAG="$(git -C "${ROOT_DIR}" rev-parse --short HEAD)-$(date -u +%Y%m%d%H%M%S)"
    fi
    set_terraform_vars

    local registry="${ACCOUNT_ID}.dkr.ecr.${AWS_REGION}.amazonaws.com"
    local repositories=(
        "${PROJECT_NAME}-${ENVIRONMENT}-submit-audio"
        "${PROJECT_NAME}-${ENVIRONMENT}-confirm-upload"
        "${PROJECT_NAME}-${ENVIRONMENT}-audio-processing"
        "${PROJECT_NAME}-${ENVIRONMENT}-start-evaluation"
        "${PROJECT_NAME}-${ENVIRONMENT}-task-callback"
        "${PROJECT_NAME}-${ENVIRONMENT}-transcription-caller"
        "${PROJECT_NAME}-${ENVIRONMENT}-moderation-caller"
        "${PROJECT_NAME}-${ENVIRONMENT}-task-events"
    )
    # One consolidated Dockerfile builds every binary in a single cargo
    # invocation; each entry selects a runtime stage with --target.
    local targets=(
        "submit-audio"
        "confirm-upload"
        "audio-processing"
        "start-evaluation"
        "task-callback"
        "transcription-caller"
        "moderation-caller"
        "task-events"
    )
    local bootstrap_targets=(
        -target=aws_ecr_repository.submit_audio -target=aws_ecr_repository_policy.submit_audio_lambda_pull -target=aws_ecr_lifecycle_policy.submit_audio
        -target=aws_ecr_repository.confirm_upload -target=aws_ecr_repository_policy.confirm_upload_lambda_pull -target=aws_ecr_lifecycle_policy.confirm_upload
        -target=aws_ecr_repository.audio_processing -target=aws_ecr_repository_policy.audio_processing_lambda_pull -target=aws_ecr_lifecycle_policy.audio_processing
        -target=aws_ecr_repository.start_evaluation -target=aws_ecr_repository_policy.start_evaluation_lambda_pull -target=aws_ecr_lifecycle_policy.start_evaluation
        -target=aws_ecr_repository.task_callback -target=aws_ecr_repository_policy.task_callback_lambda_pull -target=aws_ecr_lifecycle_policy.task_callback
        -target=aws_ecr_repository.transcription_caller -target=aws_ecr_repository_policy.transcription_caller_lambda_pull -target=aws_ecr_lifecycle_policy.transcription_caller
        -target=aws_ecr_repository.moderation_caller -target=aws_ecr_repository_policy.moderation_caller_lambda_pull -target=aws_ecr_lifecycle_policy.moderation_caller
        -target=aws_ecr_repository.task_events -target=aws_ecr_repository_policy.task_events_lambda_pull -target=aws_ecr_lifecycle_policy.task_events
    )
    local approve_args=()
    if [[ "${AUTO_APPROVE}" == true ]]; then approve_args=(-auto-approve); fi

    printf 'Bootstrapping eight ECR repositories with immutable deployment tag: %s\n' "${IMAGE_TAG}"
    terraform -chdir="${TERRAFORM_DIR}" init -input=false

    # Apply moved.tf state addresses before targetting repositories. This is
    # refresh-only, so it cannot create or update infrastructure before all
    # eight images exist; on a first deployment it is a no-op state migration.
    terraform -chdir="${TERRAFORM_DIR}" apply "${terraform_vars[@]}" "${approve_args[@]}" -refresh-only
    terraform -chdir="${TERRAFORM_DIR}" apply "${terraform_vars[@]}" "${approve_args[@]}" "${bootstrap_targets[@]}"

    local repository
    for repository in "${repositories[@]}"; do
        local image_count
        image_count="$(aws ecr batch-get-image --region "${AWS_REGION}" --repository-name "${repository}" --image-ids imageTag="${IMAGE_TAG}" --query 'length(images)' --output text)"
        if [[ "${image_count}" != 0 ]]; then
            printf 'Image tag already exists in %s; choose a new immutable tag: %s\n' "${repository}" "${IMAGE_TAG}" >&2
            exit 1
        fi
    done

    aws ecr get-login-password --region "${AWS_REGION}" | docker login --username AWS --password-stdin "${registry}"
    local index
    for index in "${!repositories[@]}"; do
        local image_uri="${registry}/${repositories[${index}]}:${IMAGE_TAG}"
        printf 'Building and pushing %s\n' "${image_uri}"
        docker build --platform linux/amd64 --provenance=false --file "${ROOT_DIR}/backend/Dockerfile.lambda" --target "${targets[${index}]}" --tag "${image_uri}" "${ROOT_DIR}"
        docker push "${image_uri}"
    done

    # This is the authoritative full apply. Every image tag is passed explicitly;
    # Terraform defaults are intentionally not used for deployed Lambda images.
    terraform -chdir="${TERRAFORM_DIR}" apply "${terraform_vars[@]}" "${approve_args[@]}"

    local output_name function_name
    for output_name in submit_audio_function_name confirm_upload_function_name audio_processing_function_name start_evaluation_function_name task_callback_function_name transcription_caller_function_name moderation_caller_function_name task_events_function_name; do
        function_name="$(terraform -chdir="${TERRAFORM_DIR}" output -raw "${output_name}")"
        aws lambda wait function-updated-v2 --region "${AWS_REGION}" --function-name "${function_name}"
        printf 'Lambda updated: %s\n' "${function_name}"
    done
    printf 'Full deployment complete. API endpoint: %s\n' "$(terraform -chdir="${TERRAFORM_DIR}" output -raw api_endpoint)"
}

load_api_endpoint() {
    if [[ -z "${AUDIO_MODERATION_API_ENDPOINT:-}" ]]; then
        AUDIO_MODERATION_API_ENDPOINT="$(terraform -chdir="${TERRAFORM_DIR}" output -raw api_endpoint)"
        export AUDIO_MODERATION_API_ENDPOINT
    fi
}

load_pipeline_database_environment() {
    AUDIO_MODERATION_TENANT_ID="${AUDIO_MODERATION_TENANT_ID:-$(terraform -chdir="${TERRAFORM_DIR}" output -raw tenant_id)}"
    export AUDIO_MODERATION_TENANT_ID
    if [[ -z "${DATABASE_URL:-}" ]]; then
        DATABASE_URL="$(aws ssm get-parameter --region "${AWS_REGION}" --name "${DATABASE_URL_PARAMETER}" --with-decryption --query 'Parameter.Value' --output text)"
        export DATABASE_URL
    fi
}

load_evaluation_environment() {
    load_api_endpoint
    load_pipeline_database_environment
}

load_task_events_endpoint() {
    validate_task_events_endpoint
    AUDIO_MODERATION_TASK_EVENTS_ENDPOINT="${TASK_EVENTS_ENDPOINT}"
    export AUDIO_MODERATION_TASK_EVENTS_ENDPOINT
}

fixture_run_id() {
    printf '%s-%s-%s' "$(date -u +%Y%m%d%H%M%S)" "$$" "${RANDOM}"
}

FIXTURE_BUCKET=""
FIXTURE_KEYS=()
FIXTURE_OBJECT_URIS=()
FIXTURE_TEMP_FILES=()
FIXTURE_TASK_IDS=()
FIXTURES_SAFE_TO_DELETE=false
cleanup_fixtures() {
    local object_uri temp_file task_id
    for temp_file in "${FIXTURE_TEMP_FILES[@]}"; do
        rm -f "${temp_file}"
    done
    if [[ "${FIXTURES_SAFE_TO_DELETE}" == true ]]; then
        for object_uri in "${FIXTURE_OBJECT_URIS[@]}"; do
            aws s3 rm "${object_uri}" --region "${AWS_REGION}" >/dev/null 2>&1 || printf 'Could not remove completed-test fixture: %s\n' "${object_uri}" >&2
        done
    elif [[ ${#FIXTURE_OBJECT_URIS[@]} -gt 0 ]]; then
        printf 'Retaining fixtures because asynchronous work may still use them:\n' >&2
        printf '  %s\n' "${FIXTURE_OBJECT_URIS[@]}" >&2
    fi
    # Pipeline tasks are database fixtures as well as S3 fixtures. The URL is
    # passed only to psql and is never printed.
    for task_id in "${FIXTURE_TASK_IDS[@]}"; do
        psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 -q -c "DELETE FROM pipeline_tasks WHERE task_id = ${task_id}" >/dev/null 2>&1 || printf 'Could not remove completed-test pipeline task: %s\n' "${task_id}" >&2
    done
}

provision_input_fixtures() {
    validate_audio_file
    FIXTURE_BUCKET="$(terraform -chdir="${TERRAFORM_DIR}" output -raw uploads_bucket_name)"
    local run_id
    run_id="$(fixture_run_id)"
    FIXTURE_KEYS=(
        "reviews/integration-tests/${run_id}/first.wav"
        "reviews/integration-tests/${run_id}/second.wav"
    )
    FIXTURE_OBJECT_URIS=(
        "s3://${FIXTURE_BUCKET}/${FIXTURE_KEYS[0]}"
        "s3://${FIXTURE_BUCKET}/${FIXTURE_KEYS[1]}"
    )
    local key
    for key in "${FIXTURE_KEYS[@]}"; do
        aws s3 cp "${AUDIO_FILE}" "s3://${FIXTURE_BUCKET}/${key}" --region "${AWS_REGION}"
    done
    AUDIO_MODERATION_TEST_AUDIO_S3_URIS="$(jq -cn --arg bucket "${FIXTURE_BUCKET}" --arg first "${FIXTURE_KEYS[0]}" --arg second "${FIXTURE_KEYS[1]}" '["s3://" + $bucket + "/" + $first, "s3://" + $bucket + "/" + $second]')"
    export AUDIO_MODERATION_TEST_AUDIO_S3_URIS
}

wait_for_execution_success() {
    local execution_arn="$1"
    local status
    for _ in {1..60}; do
        status="$(aws stepfunctions describe-execution --region "${AWS_REGION}" --execution-arn "${execution_arn}" --query status --output text)"
        case "${status}" in
            SUCCEEDED) return 0 ;;
            FAILED|TIMED_OUT|ABORTED)
                printf 'Test Step Functions execution ended as %s: %s\n' "${status}" "$(aws stepfunctions describe-execution --region "${AWS_REGION}" --execution-arn "${execution_arn}" --query cause --output text)" >&2
                return 1
                ;;
        esac
        sleep 2
    done
    printf 'Test Step Functions execution did not complete within two minutes: %s\n' "${execution_arn}" >&2
    return 1
}

run_suite() {
    case "${SUITE}" in
        review-submit)
            preflight review-submit
            load_api_endpoint
            cargo test --manifest-path "${BACKEND_DIR}/Cargo.toml" --config "${BACKEND_DIR}/.cargo/config.toml" --package submit-audio-lambda --test deployed submits_review_against_aws -- --ignored --nocapture
            ;;
        review-confirmation)
            preflight review-confirmation
            load_api_endpoint
            cargo test --manifest-path "${BACKEND_DIR}/Cargo.toml" --config "${BACKEND_DIR}/.cargo/config.toml" --package submit-audio-lambda --test deployed confirms_uploaded_review_against_aws -- --ignored --nocapture
            ;;
        evaluation-ingress)
            preflight evaluation-ingress
            unset AUDIO_MODERATION_TEST_AUDIO_S3_URIS
            load_evaluation_environment
            cargo test --manifest-path "${BACKEND_DIR}/Cargo.toml" --config "${BACKEND_DIR}/.cargo/config.toml" --package start-evaluation-lambda --test deployed evaluation_ingress_ -- --ignored --nocapture
            ;;
        evaluation-dispatch)
            preflight evaluation-dispatch
            load_evaluation_environment
            # Dispatch assertions finish before the production workflows do.
            # Retain and report their fixtures for those asynchronous executions.
            trap cleanup_fixtures EXIT
            provision_input_fixtures
            cargo test --manifest-path "${BACKEND_DIR}/Cargo.toml" --config "${BACKEND_DIR}/.cargo/config.toml" --package start-evaluation-lambda --test deployed evaluation_dispatch_ -- --ignored --nocapture
            ;;
        audio-conversion)
            preflight audio-conversion
            local artifacts_bucket run_id input_key output_key payload_file response_file function_name
            load_evaluation_environment
            require_command psql
            FIXTURE_BUCKET="$(terraform -chdir="${TERRAFORM_DIR}" output -raw uploads_bucket_name)"
            artifacts_bucket="$(terraform -chdir="${TERRAFORM_DIR}" output -raw artifacts_bucket_name)"
            run_id="$(fixture_run_id)"
            input_key="reviews/integration-tests/${run_id}/audio.wav"
            output_key="evaluations/integration-tests/${run_id}/stitched.wav"
            FIXTURE_KEYS=("${input_key}")
            FIXTURE_OBJECT_URIS=(
                "s3://${FIXTURE_BUCKET}/${input_key}"
                "s3://${artifacts_bucket}/${output_key}"
            )
            # Direct invocation is synchronous, so these objects can be removed
            # even if upload, mktemp, or Lambda invocation fails.
            FIXTURES_SAFE_TO_DELETE=true
            trap cleanup_fixtures EXIT
            aws s3 cp "${AUDIO_FILE}" "s3://${FIXTURE_BUCKET}/${input_key}" --region "${AWS_REGION}"
            # The worker now requires a persisted numeric pipeline task. Create
            # its complete step fixture transactionally and delete it in the
            # suite cleanup path.
            task_id="$(psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 -Atqc "WITH task AS (INSERT INTO pipeline_tasks (tenant_id, idempotency_key) VALUES ('${TENANT_ID}', 'audio-conversion-${run_id}') RETURNING task_id), audio AS (INSERT INTO audio_processing_tasks (task_id) SELECT task_id FROM task), transcription AS (INSERT INTO transcription_tasks (task_id) SELECT task_id FROM task), moderation AS (INSERT INTO moderation_tasks (task_id) SELECT task_id FROM task), input AS (INSERT INTO pipeline_task_inputs (task_id, sequence, audio_s3_uri) SELECT task_id, 0, 's3://${FIXTURE_BUCKET}/${input_key}' FROM task) SELECT task_id FROM task")"
            FIXTURE_TASK_IDS=("${task_id}")
            payload_file="$(mktemp)"
            FIXTURE_TEMP_FILES=("${payload_file}")
            response_file="$(mktemp)"
            FIXTURE_TEMP_FILES+=("${response_file}")
            jq -n --arg job_id "${task_id}" --arg input "s3://${FIXTURE_BUCKET}/${input_key}" --arg output "s3://${artifacts_bucket}/${output_key}" '{jobId: $job_id, files: [{s3Uri: $input, sequence: 0}], outputS3Uri: $output}' >"${payload_file}"
            function_name="$(terraform -chdir="${TERRAFORM_DIR}" output -raw audio_processing_function_name)"
            aws lambda invoke --region "${AWS_REGION}" --function-name "${function_name}" --cli-binary-format raw-in-base64-out --payload "file://${payload_file}" "${response_file}" >/dev/null
            jq -e --arg output "s3://${artifacts_bucket}/${output_key}" '.stitchedS3Uri == $output' "${response_file}" >/dev/null
            aws s3api head-object --region "${AWS_REGION}" --bucket "${artifacts_bucket}" --key "${output_key}" >/dev/null
            ;;
        task-events)
            preflight task-events
            load_pipeline_database_environment
            load_task_events_endpoint
            cargo test --manifest-path "${BACKEND_DIR}/Cargo.toml" --config "${BACKEND_DIR}/.cargo/config.toml" --package task-events-lambda --test deployed replays_live_delivers_and_cleans_up_task_events -- --ignored --nocapture
            ;;
        task-callback)
            printf 'task-callback is unsupported: a valid callback now requires an ASR-created persisted task-token digest. The legacy test state machine cannot seed that digest before it receives its generated token.\n' >&2
            return 1
            ;;
        transcription-caller)
            printf 'transcription-caller is unsupported: the deployed caller needs a compatible external transcription endpoint plus a database task and a real Step Functions task token. The checked-in socialguard-models application contract must not be changed to supply that harness.\n' >&2
            return 1
            ;;
        moderation-caller)
            printf 'moderation-caller is unsupported: the deployed caller needs a completed transcription plus a database task and a real Step Functions task token.\n' >&2
            return 1
            ;;
        evaluation-e2e)
            preflight evaluation-e2e
            load_evaluation_environment
            load_task_events_endpoint
            # Keep partial or failed asynchronous fixtures, but report every
            # possible URI so they can be investigated or removed manually.
            trap cleanup_fixtures EXIT
            provision_input_fixtures
            cargo test --manifest-path "${BACKEND_DIR}/Cargo.toml" --config "${BACKEND_DIR}/.cargo/config.toml" --package start-evaluation-lambda --test deployed completes_a_fresh_evaluation_and_replays_without_another_attempt -- --ignored --nocapture
            FIXTURES_SAFE_TO_DELETE=true
            ;;
    esac
}

case "${ACTION}" in
    preflight) preflight "${SUITE:-full}" ;;
    deploy) deploy ;;
    test) run_suite ;;
    # Validate the selected suite before mutating AWS resources. The WebSocket
    # endpoint is intentionally deferred because this deployment creates it.
    all) preflight "${SUITE}" true; deploy; run_suite ;;
esac
