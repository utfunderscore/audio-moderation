#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
TERRAFORM_DIR="${ROOT_DIR}/terraform"

AWS_REGION="${AWS_REGION:-eu-west-2}"
PROJECT_NAME="${PROJECT_NAME:-audio-moderation}"
ENVIRONMENT="${ENVIRONMENT:-dev}"
DATABASE_URL_PARAMETER="${DATABASE_URL_PARAMETER:-}"
TENANT_ID="${TENANT_ID:-default}"
IMAGE_TAG="${IMAGE_TAG:-}"
AUTO_APPROVE=false
RUN_INTEGRATION_TEST=true

usage() {
    cat <<'EOF'
Usage: ./deploy.sh [options]

Build and push the Lambda images, then deploy the AWS infrastructure.

Options:
  --region REGION                 AWS region (default: eu-west-2)
  --project-name NAME             Project name (default: audio-moderation)
  --environment NAME              Deployment environment (default: dev)
  --database-parameter-name NAME  SSM parameter (default: /PROJECT/ENV/database-url)
  --tenant-id ID                  Stable tenant ID (default: default)
  --image-tag TAG                 Image tag (default: git SHA plus UTC timestamp)
  --auto-approve                  Skip Terraform approval prompts
  --skip-integration-test         Deploy without running the AWS integration test
  -h, --help                      Show this help

The same values can be supplied through AWS_REGION, PROJECT_NAME, ENVIRONMENT,
DATABASE_URL_PARAMETER, TENANT_ID, and IMAGE_TAG environment variables.
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
        --tenant-id)
            require_value "$@"
            TENANT_ID="$2"
            shift 2
            ;;
        --image-tag)
            require_value "$@"
            IMAGE_TAG="$2"
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

for command in aws cargo docker git terraform; do
    if ! command -v "${command}" >/dev/null 2>&1; then
        printf 'Required command not found: %s\n' "${command}" >&2
        exit 1
    fi
done

if [[ -z "${IMAGE_TAG}" ]]; then
    IMAGE_TAG="$(git -C "${ROOT_DIR}" rev-parse --short HEAD)-$(date -u +%Y%m%d%H%M%S)"
fi

ACCOUNT_ID="$(aws sts get-caller-identity --region "${AWS_REGION}" --query Account --output text)"
if [[ -z "${ACCOUNT_ID}" || "${ACCOUNT_ID}" == "None" ]]; then
    printf 'Unable to determine the active AWS account\n' >&2
    exit 1
fi

ECR_REPOSITORY="${PROJECT_NAME}-${ENVIRONMENT}-submit-audio"
UPLOAD_COMPLETE_ECR_REPOSITORY="${PROJECT_NAME}-${ENVIRONMENT}-upload-complete"
ECR_REGISTRY="${ACCOUNT_ID}.dkr.ecr.${AWS_REGION}.amazonaws.com"
IMAGE_URI="${ECR_REGISTRY}/${ECR_REPOSITORY}:${IMAGE_TAG}"
UPLOAD_COMPLETE_IMAGE_URI="${ECR_REGISTRY}/${UPLOAD_COMPLETE_ECR_REPOSITORY}:${IMAGE_TAG}"

terraform_args=(
    -var="aws_region=${AWS_REGION}"
    -var="project_name=${PROJECT_NAME}"
    -var="environment=${ENVIRONMENT}"
    -var="database_parameter_name=${DATABASE_URL_PARAMETER}"
    -var="tenant_id=${TENANT_ID}"
    -var="submit_audio_image_tag=${IMAGE_TAG}"
    -var="upload_complete_image_tag=${IMAGE_TAG}"
)
approve_args=()
if [[ "${AUTO_APPROVE}" == true ]]; then
    approve_args=(-auto-approve)
fi

printf 'AWS account: %s\n' "${ACCOUNT_ID}"
printf 'Environment: %s\n' "${ENVIRONMENT}"
printf 'Image: %s\n' "${IMAGE_URI}"
printf 'Image: %s\n' "${UPLOAD_COMPLETE_IMAGE_URI}"

terraform -chdir="${TERRAFORM_DIR}" init -input=false

# Both repositories must exist before their images can be pushed. The full apply
# below remains authoritative for these resources and all dependent infrastructure.
terraform -chdir="${TERRAFORM_DIR}" apply \
    "${terraform_args[@]}" \
    "${approve_args[@]}" \
    -target=aws_ecr_repository.submit_audio \
    -target=aws_ecr_repository_policy.submit_audio_lambda_pull \
    -target=aws_ecr_lifecycle_policy.submit_audio \
    -target=aws_ecr_repository.upload_complete \
    -target=aws_ecr_repository_policy.upload_complete_lambda_pull \
    -target=aws_ecr_lifecycle_policy.upload_complete

existing_image_count="$(aws ecr batch-get-image \
    --region "${AWS_REGION}" \
    --repository-name "${ECR_REPOSITORY}" \
    --image-ids imageTag="${IMAGE_TAG}" \
    --query 'length(images)' \
    --output text)"
if [[ "${existing_image_count}" != 0 ]]; then
    printf 'Image tag already exists; choose a new immutable tag: %s\n' "${IMAGE_TAG}" >&2
    exit 1
fi

upload_complete_existing_image_count="$(aws ecr batch-get-image \
    --region "${AWS_REGION}" \
    --repository-name "${UPLOAD_COMPLETE_ECR_REPOSITORY}" \
    --image-ids imageTag="${IMAGE_TAG}" \
    --query 'length(images)' \
    --output text)"
if [[ "${upload_complete_existing_image_count}" != 0 ]]; then
    printf 'Image tag already exists; choose a new immutable tag: %s\n' "${IMAGE_TAG}" >&2
    exit 1
fi

aws ecr get-login-password --region "${AWS_REGION}" \
    | docker login --username AWS --password-stdin "${ECR_REGISTRY}"

docker build \
    --platform linux/amd64 \
    --provenance=false \
    --file "${ROOT_DIR}/crates/submit-audio-lambda/Dockerfile" \
    --tag "${IMAGE_URI}" \
    "${ROOT_DIR}"
docker push "${IMAGE_URI}"
docker build \
    --platform linux/amd64 \
    --provenance=false \
    --file "${ROOT_DIR}/crates/upload-complete-lambda/Dockerfile" \
    --tag "${UPLOAD_COMPLETE_IMAGE_URI}" \
    "${ROOT_DIR}"
docker push "${UPLOAD_COMPLETE_IMAGE_URI}"

terraform -chdir="${TERRAFORM_DIR}" apply \
    "${terraform_args[@]}" \
    "${approve_args[@]}"

FUNCTION_NAME="$(terraform -chdir="${TERRAFORM_DIR}" output -raw submit_audio_function_name)"
UPLOAD_COMPLETE_FUNCTION_NAME="$(terraform -chdir="${TERRAFORM_DIR}" output -raw upload_complete_function_name)"
aws lambda wait function-updated-v2 \
    --region "${AWS_REGION}" \
    --function-name "${FUNCTION_NAME}"
aws lambda wait function-updated-v2 \
    --region "${AWS_REGION}" \
    --function-name "${UPLOAD_COMPLETE_FUNCTION_NAME}"

printf 'Deployment complete\n'
API_ENDPOINT="$(terraform -chdir="${TERRAFORM_DIR}" output -raw api_endpoint)"
printf 'API endpoint: %s\n' "${API_ENDPOINT}"

if [[ "${RUN_INTEGRATION_TEST}" == true ]]; then
    printf 'Running deployed integration test\n'
    AUDIO_MODERATION_API_ENDPOINT="${API_ENDPOINT}" \
        cargo test \
        --manifest-path "${ROOT_DIR}/Cargo.toml" \
        --package submit-audio-lambda \
        --test deployed \
        -- \
        --ignored \
        --nocapture
fi
