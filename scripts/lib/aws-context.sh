#!/usr/bin/env bash

# Shared target identity only; sourcing this file performs no remote operations.
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
TERRAFORM_DIR="${ROOT_DIR}/terraform"
BACKEND_DIR="${ROOT_DIR}/backend"
export AWS_PROFILE=admin
AWS_REGION="${AWS_REGION:-eu-west-2}"
PROJECT_NAME="${PROJECT_NAME:-audio-moderation}"
ENVIRONMENT="${ENVIRONMENT:-dev}"
DATABASE_URL_PARAMETER="${DATABASE_URL_PARAMETER:-}"

require_value() {
    if [[ $# -lt 2 || -z "$2" || "$2" == -* ]]; then
        printf 'Missing value for %s\n' "$1" >&2
        exit 2
    fi
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'Required command not found: %s\n' "$1" >&2
        exit 1
    fi
}

configure_aws_context() {
    export AWS_REGION
    DATABASE_URL_PARAMETER="${DATABASE_URL_PARAMETER:-/${PROJECT_NAME}/${ENVIRONMENT}/database-url}"
}

validate_aws_context() {
    require_command aws
    ACCOUNT_ID="$(AWS_PROFILE=admin aws sts get-caller-identity --region "${AWS_REGION}" --query Account --output text)"
    if [[ -z "${ACCOUNT_ID}" || "${ACCOUNT_ID}" == "None" ]]; then
        printf 'Unable to determine the AWS account for AWS_PROFILE=admin.\n' >&2
        exit 1
    fi
    printf 'AWS_PROFILE=admin account: %s\nAWS target region: %s\n' "${ACCOUNT_ID}" "${AWS_REGION}"
}

validate_secure_parameter() {
    local parameter_name="$1" parameter_type
    parameter_type="$(AWS_PROFILE=admin aws ssm get-parameter --region "${AWS_REGION}" --name "${parameter_name}" --query 'Parameter.Type' --output text)"
    if [[ "${parameter_type}" != "SecureString" ]]; then
        printf 'Required encrypted SSM parameter is not a SecureString: %s\n' "${parameter_name}" >&2
        exit 1
    fi
    printf 'Validated encrypted SSM parameter: %s\n' "${parameter_name}"
}
