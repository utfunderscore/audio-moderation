#!/usr/bin/env bash

terraform_vars=()
set_terraform_vars() {
    terraform_vars=(
        -var="aws_region=${AWS_REGION}"
        -var="project_name=${PROJECT_NAME}"
        -var="environment=${ENVIRONMENT}"
        -var="tenant_id=${TENANT_ID}"
        -var="database_parameter_name=${DATABASE_URL_PARAMETER}"
        -var="turnstile_secret_key_parameter_name=${TURNSTILE_SECRET_KEY_PARAMETER}"
        -var="turnstile_allowed_hostnames=${TURNSTILE_ALLOWED_HOSTNAMES}"
        -var="modal_proxy_token_id_parameter_name=${MODAL_PROXY_TOKEN_ID_PARAMETER}"
        -var="modal_proxy_token_secret_parameter_name=${MODAL_PROXY_TOKEN_SECRET_PARAMETER}"
        -var="transcription_endpoint_url=${TRANSCRIPTION_ENDPOINT_URL}"
        -var="modal_endpoint_url=${MODAL_ENDPOINT_URL}"
        -var="modal_workspace_id=${MODAL_WORKSPACE_ID}"
        -var="enable_test_resources=true"
        -var="enable_cloudflare_proxy=${ENABLE_CLOUDFLARE_PROXY}"
        -var="cloudflare_zone_name=${CLOUDFLARE_ZONE_NAME}"
        -var="public_api_domain_name=${PUBLIC_API_DOMAIN_NAME}"
        -var="task_events_domain_name=${TASK_EVENTS_DOMAIN_NAME}"
    )
    local target address
    for target in "${LAMBDA_TARGETS[@]}"; do
        address="${target//-/_}"
        terraform_vars+=(-var="${address}_image_tag=${IMAGE_TAG}")
    done
}

deploy() {
    deployment_preflight
    if [[ -z "${IMAGE_TAG}" ]]; then
        IMAGE_TAG="$(git -C "${ROOT_DIR}" rev-parse --short HEAD)-$(date -u +%Y%m%d%H%M%S)"
    fi
    if [[ "${IMAGE_TAG}" == latest ]]; then
        printf 'Use an immutable image tag, never latest.\n' >&2
        exit 2
    fi
    set_terraform_vars
    local target address function_name
    local bootstrap_targets=() approve_args=()
    for target in "${LAMBDA_TARGETS[@]}"; do
        address="${target//-/_}"
        bootstrap_targets+=(
            -target="aws_ecr_repository.${address}"
            -target="aws_ecr_repository_policy.${address}_lambda_pull"
            -target="aws_ecr_lifecycle_policy.${address}"
        )
    done
    if [[ "${AUTO_APPROVE}" == true ]]; then approve_args=(-auto-approve); fi

    printf 'Bootstrapping eight ECR repositories with immutable deployment tag: %s\n' "${IMAGE_TAG}"
    AWS_PROFILE=admin terraform -chdir="${TERRAFORM_DIR}" init -input=false
    # Apply moved state addresses before targeting repositories. Refresh-only
    # cannot create/update infrastructure before all eight images exist.
    AWS_PROFILE=admin terraform -chdir="${TERRAFORM_DIR}" apply "${terraform_vars[@]}" "${approve_args[@]}" -refresh-only
    AWS_PROFILE=admin terraform -chdir="${TERRAFORM_DIR}" apply "${terraform_vars[@]}" "${approve_args[@]}" "${bootstrap_targets[@]}"
    publish_images
    # Authoritative full apply: explicitly use the same tag for every Lambda.
    AWS_PROFILE=admin terraform -chdir="${TERRAFORM_DIR}" apply "${terraform_vars[@]}" "${approve_args[@]}"
    for target in "${LAMBDA_TARGETS[@]}"; do
        address="${target//-/_}"
        function_name="$(deployment_output "${address}_function_name")"
        AWS_PROFILE=admin aws lambda wait function-updated-v2 --region "${AWS_REGION}" --function-name "${function_name}"
        printf 'Lambda updated: %s\n' "${function_name}"
    done
    printf 'Full deployment complete. API endpoint: %s\n' "$(deployment_output api_endpoint)"
    AWS_PROFILE=admin UI_ENV_DIR="${ROOT_DIR}/ui" "${ROOT_DIR}/ui/scripts/sync-deployment-env.sh"
}
