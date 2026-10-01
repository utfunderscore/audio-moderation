#!/usr/bin/env bash

# One list drives repository addresses, Docker targets, image vars, and waiters.
LAMBDA_TARGETS=(submit-audio confirm-upload audio-processing start-evaluation task-callback transcription-caller moderation-caller task-events)

publish_images() {
    local registry="${ACCOUNT_ID}.dkr.ecr.${AWS_REGION}.amazonaws.com"
    local target repository image_count image_uri
    # Check every repository before publishing any image under the shared tag.
    for target in "${LAMBDA_TARGETS[@]}"; do
        repository="${PROJECT_NAME}-${ENVIRONMENT}-${target}"
        image_count="$(AWS_PROFILE=admin aws ecr batch-get-image --region "${AWS_REGION}" --repository-name "${repository}" --image-ids imageTag="${IMAGE_TAG}" --query 'length(images)' --output text)"
        if [[ "${image_count}" != 0 ]]; then
            printf 'Image tag already exists in %s; choose a new immutable tag: %s\n' "${repository}" "${IMAGE_TAG}" >&2
            exit 1
        fi
    done
    AWS_PROFILE=admin aws ecr get-login-password --region "${AWS_REGION}" | docker login --username AWS --password-stdin "${registry}"
    for target in "${LAMBDA_TARGETS[@]}"; do
        image_uri="${registry}/${PROJECT_NAME}-${ENVIRONMENT}-${target}:${IMAGE_TAG}"
        printf 'Building and pushing %s\n' "${image_uri}"
        # Repository-root context; one consolidated Dockerfile selects runtime stages.
        docker build --platform linux/amd64 --provenance=false --file "${ROOT_DIR}/backend/Dockerfile.lambda" --target "${target}" --tag "${image_uri}" "${ROOT_DIR}"
        docker push "${image_uri}"
    done
}
