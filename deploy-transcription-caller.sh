#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
TERRAFORM_DIR="${ROOT_DIR}/terraform"

export AWS_PROFILE=admin

AWS_REGION="${AWS_REGION:-eu-west-2}"
PROJECT_NAME="${PROJECT_NAME:-audio-moderation}"
ENVIRONMENT="${ENVIRONMENT:-dev}"
DATABASE_URL_PARAMETER="${DATABASE_URL_PARAMETER:-}"
MODAL_PROXY_TOKEN_ID_PARAMETER="${MODAL_PROXY_TOKEN_ID_PARAMETER:-}"
MODAL_PROXY_TOKEN_SECRET_PARAMETER="${MODAL_PROXY_TOKEN_SECRET_PARAMETER:-}"
TRANSCRIPTION_ENDPOINT_URL="${TRANSCRIPTION_ENDPOINT_URL:-https://utfunderscore--socialguard-transcription-api-submit-tran-0a77af.modal.run}"
MODAL_WORKSPACE_ID="${MODAL_WORKSPACE_ID:-ac-k4lbrkEynY351mickkxfRh}"
IMAGE_TAG="${IMAGE_TAG:-}"
AUDIO_FILE="${AUDIO_FILE:-${ROOT_DIR}/sample_071.mp3}"
AUTO_APPROVE=false
RUN_INTEGRATION_TEST=true

usage() {
    cat <<'EOF'
Usage: ./deploy-transcription-caller.sh [options]

Build, upload, and deploy the transcription-caller and task-callback Lambdas,
then run the ignored StartEvaluation pipeline suite.

Options:
  --region REGION                         AWS region (default: eu-west-2)
  --project-name NAME                     Project name (default: audio-moderation)
  --environment NAME                      Deployment environment (default: dev)
  --database-parameter-name NAME          Database URL SSM parameter
  --modal-token-id-parameter-name NAME    Modal proxy token ID SSM parameter
  --modal-token-secret-parameter-name NAME
                                           Modal proxy token secret SSM parameter
  --transcription-endpoint-url URL         Override the default Modal transcription endpoint
  --modal-workspace-id ID                  Modal workspace ID (default: utfunderscore workspace)
  --image-tag TAG                          Image tag (default: git SHA plus UTC timestamp)
  --audio-file FILE                        Integration fixture (default: sample_071.mp3)
  --auto-approve                           Skip Terraform approval prompts
  --skip-integration-test                  Deploy without running the ignored suite
  -h, --help                               Show this help

The same values can be supplied through AWS_REGION, PROJECT_NAME, ENVIRONMENT,
DATABASE_URL_PARAMETER, MODAL_PROXY_TOKEN_ID_PARAMETER,
MODAL_PROXY_TOKEN_SECRET_PARAMETER, TRANSCRIPTION_ENDPOINT_URL,
MODAL_WORKSPACE_ID, IMAGE_TAG, and AUDIO_FILE. AWS commands always use the
local admin profile.
EOF
}

require_value() {
    if [[ $# -lt 2 ]] || [[ -z "$2" || "$2" == -* ]]; then
        printf 'Missing value for %s\n' "$1" >&2
        usage >&2
        exit 2
    fi
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --region)
            require_value "$@"
            AWS_REGION="$2"
            shift 2
            ;;
        --project-name)
            require_value "$@"
            PROJECT_NAME="$2"
            shift 2
            ;;
        --environment)
            require_value "$@"
            ENVIRONMENT="$2"
            shift 2
            ;;
        --database-parameter-name)
            require_value "$@"
            DATABASE_URL_PARAMETER="$2"
            shift 2
            ;;
        --modal-token-id-parameter-name)
            require_value "$@"
            MODAL_PROXY_TOKEN_ID_PARAMETER="$2"
            shift 2
            ;;
        --modal-token-secret-parameter-name)
            require_value "$@"
            MODAL_PROXY_TOKEN_SECRET_PARAMETER="$2"
            shift 2
            ;;
        --transcription-endpoint-url)
            require_value "$@"
            TRANSCRIPTION_ENDPOINT_URL="$2"
            shift 2
            ;;
        --modal-workspace-id)
            require_value "$@"
            MODAL_WORKSPACE_ID="$2"
            shift 2
            ;;
        --image-tag)
            require_value "$@"
            IMAGE_TAG="$2"
            shift 2
            ;;
        --audio-file)
            require_value "$@"
            AUDIO_FILE="$2"
            shift 2
            ;;
        --auto-approve)
            AUTO_APPROVE=true
            shift
            ;;
        --skip-integration-test)
            RUN_INTEGRATION_TEST=false
            shift
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

if [[ -z "${DATABASE_URL_PARAMETER}" ]]; then
    DATABASE_URL_PARAMETER="/${PROJECT_NAME}/${ENVIRONMENT}/database-url"
fi
if [[ -z "${MODAL_PROXY_TOKEN_ID_PARAMETER}" ]]; then
    MODAL_PROXY_TOKEN_ID_PARAMETER="/${PROJECT_NAME}/${ENVIRONMENT}/modal-proxy-token-id"
fi
if [[ -z "${MODAL_PROXY_TOKEN_SECRET_PARAMETER}" ]]; then
    MODAL_PROXY_TOKEN_SECRET_PARAMETER="/${PROJECT_NAME}/${ENVIRONMENT}/modal-proxy-token-secret"
fi

if [[ -z "${TRANSCRIPTION_ENDPOINT_URL}" ]]; then
    printf 'TRANSCRIPTION_ENDPOINT_URL or --transcription-endpoint-url is required\n' >&2
    exit 2
fi
for command in aws cargo docker git terraform; do
    if ! command -v "${command}" >/dev/null 2>&1; then
        printf 'Required command not found: %s\n' "${command}" >&2
        exit 1
    fi
done

if [[ "${RUN_INTEGRATION_TEST}" == true ]]; then
    if [[ "${AUDIO_FILE}" != /* ]]; then
        AUDIO_FILE="$(pwd -P)/${AUDIO_FILE}"
    fi
    if [[ ! -f "${AUDIO_FILE}" || ! -r "${AUDIO_FILE}" || ! -s "${AUDIO_FILE}" ]]; then
        printf 'Audio file must be a regular, readable, nonempty file: %s\n' "${AUDIO_FILE}" >&2
        exit 1
    fi
fi

if [[ -z "${IMAGE_TAG}" ]]; then
    IMAGE_TAG="$(git -C "${ROOT_DIR}" rev-parse --short HEAD)-$(date -u +%Y%m%d%H%M%S)"
fi

ACCOUNT_ID="$(aws sts get-caller-identity --region "${AWS_REGION}" --query Account --output text)"
if [[ -z "${ACCOUNT_ID}" || "${ACCOUNT_ID}" == "None" ]]; then
    printf 'Unable to determine the admin AWS account\n' >&2
    exit 1
fi

AUDIO_PROCESSING_FUNCTION="${PROJECT_NAME}-${ENVIRONMENT}-audio-processing"
AUDIO_PROCESSING_IMAGE_URI="$(aws lambda get-function \
    --region "${AWS_REGION}" \
    --function-name "${AUDIO_PROCESSING_FUNCTION}" \
    --query 'Code.ImageUri' \
    --output text 2>/dev/null || true)"
if [[ -z "${AUDIO_PROCESSING_IMAGE_URI}" || "${AUDIO_PROCESSING_IMAGE_URI}" == "None" || "${AUDIO_PROCESSING_IMAGE_URI}" != *:* ]]; then
    printf 'The deployed audio-processing Lambda is required before deploying its transcription workflow integration: %s\n' "${AUDIO_PROCESSING_FUNCTION}" >&2
    exit 1
fi
AUDIO_PROCESSING_IMAGE_TAG="${AUDIO_PROCESSING_IMAGE_URI##*:}"

ECR_REPOSITORY="${PROJECT_NAME}-${ENVIRONMENT}-transcription-caller"
TASK_CALLBACK_ECR_REPOSITORY="${PROJECT_NAME}-${ENVIRONMENT}-task-callback"
ECR_REGISTRY="${ACCOUNT_ID}.dkr.ecr.${AWS_REGION}.amazonaws.com"
IMAGE_URI="${ECR_REGISTRY}/${ECR_REPOSITORY}:${IMAGE_TAG}"
TASK_CALLBACK_IMAGE_URI="${ECR_REGISTRY}/${TASK_CALLBACK_ECR_REPOSITORY}:${IMAGE_TAG}"

terraform_args=(
    -var="aws_region=${AWS_REGION}"
    -var="project_name=${PROJECT_NAME}"
    -var="environment=${ENVIRONMENT}"
    -var="database_parameter_name=${DATABASE_URL_PARAMETER}"
    -var="modal_proxy_token_id_parameter_name=${MODAL_PROXY_TOKEN_ID_PARAMETER}"
    -var="modal_proxy_token_secret_parameter_name=${MODAL_PROXY_TOKEN_SECRET_PARAMETER}"
    -var="transcription_endpoint_url=${TRANSCRIPTION_ENDPOINT_URL}"
    -var="modal_workspace_id=${MODAL_WORKSPACE_ID}"
    -var="audio_processing_image_tag=${AUDIO_PROCESSING_IMAGE_TAG}"
    -var="task_callback_image_tag=${IMAGE_TAG}"
    -var="transcription_caller_image_tag=${IMAGE_TAG}"
)
approve_args=()
if [[ "${AUTO_APPROVE}" == true ]]; then
    approve_args=(-auto-approve)
fi

printf 'AWS account: %s\n' "${ACCOUNT_ID}"
printf 'Environment: %s\n' "${ENVIRONMENT}"
printf 'Transcription image: %s\n' "${IMAGE_URI}"
printf 'Task callback image: %s\n' "${TASK_CALLBACK_IMAGE_URI}"

terraform -chdir="${TERRAFORM_DIR}" init -input=false

# Bootstrap the repository before pushing. The targeted workflow apply below
# avoids changing unrelated image-based Lambdas whose tags are managed elsewhere.
terraform -chdir="${TERRAFORM_DIR}" apply \
    "${terraform_args[@]}" \
    "${approve_args[@]}" \
    -target=aws_ecr_repository.transcription_caller \
    -target=aws_ecr_repository_policy.transcription_caller_lambda_pull \
    -target=aws_ecr_lifecycle_policy.transcription_caller \
    -target=aws_ecr_repository.task_callback \
    -target=aws_ecr_repository_policy.task_callback_lambda_pull \
    -target=aws_ecr_lifecycle_policy.task_callback

for repository in "${ECR_REPOSITORY}" "${TASK_CALLBACK_ECR_REPOSITORY}"; do
    existing_image_count="$(aws ecr batch-get-image \
        --region "${AWS_REGION}" \
        --repository-name "${repository}" \
        --image-ids imageTag="${IMAGE_TAG}" \
        --query 'length(images)' \
        --output text)"
    if [[ "${existing_image_count}" != 0 ]]; then
        printf 'Image tag already exists in %s; choose a new immutable tag: %s\n' "${repository}" "${IMAGE_TAG}" >&2
        exit 1
    fi
done

aws ecr get-login-password --region "${AWS_REGION}" \
    | docker login --username AWS --password-stdin "${ECR_REGISTRY}"

docker build \
    --platform linux/amd64 \
    --provenance=false \
    --file "${ROOT_DIR}/crates/transcription-caller-lambda/Dockerfile" \
    --tag "${IMAGE_URI}" \
    "${ROOT_DIR}"
docker push "${IMAGE_URI}"

docker build \
    --platform linux/amd64 \
    --provenance=false \
    --file "${ROOT_DIR}/crates/task-callback-lambda/Dockerfile" \
    --tag "${TASK_CALLBACK_IMAGE_URI}" \
    "${ROOT_DIR}"
docker push "${TASK_CALLBACK_IMAGE_URI}"

terraform -chdir="${TERRAFORM_DIR}" apply \
    "${terraform_args[@]}" \
    "${approve_args[@]}" \
    -target=aws_lambda_function.transcription_caller \
    -target=aws_lambda_function.task_callback \
    -target=aws_iam_role_policy.modal_stitched_audio_reader \
    -target=aws_iam_role_policy.audio_processing_state_machine \
    -target=aws_sfn_state_machine.audio_processing

# Targeted applies do not persist newly added root outputs. Refreshing state is
# configuration-safe and makes the test runner's Terraform outputs available.
terraform -chdir="${TERRAFORM_DIR}" apply \
    "${terraform_args[@]}" \
    -refresh-only \
    -auto-approve

FUNCTION_NAME="$(terraform -chdir="${TERRAFORM_DIR}" output -raw transcription_caller_function_name)"
TASK_CALLBACK_FUNCTION_NAME="${PROJECT_NAME}-${ENVIRONMENT}-task-callback"
aws lambda wait function-updated-v2 \
    --region "${AWS_REGION}" \
    --function-name "${FUNCTION_NAME}"
aws lambda wait function-updated-v2 \
    --region "${AWS_REGION}" \
    --function-name "${TASK_CALLBACK_FUNCTION_NAME}"

printf 'Transcription-caller deployment complete\n'
printf 'Lambda function: %s\n' "${FUNCTION_NAME}"
printf 'Task callback Lambda: %s\n' "${TASK_CALLBACK_FUNCTION_NAME}"

if [[ "${RUN_INTEGRATION_TEST}" == true ]]; then
    printf 'Running deployed StartEvaluation pipeline suite\n'
    "${ROOT_DIR}/run-start-evaluation-tests.sh" --audio-file "${AUDIO_FILE}"
fi
