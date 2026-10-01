#!/usr/bin/env bash

suite_preflight() {
    require_command cargo
    validate_secure_parameter "${DATABASE_URL_PARAMETER}"
    validate_secure_parameter "${TURNSTILE_SECRET_KEY_PARAMETER}"
    validate_task_events_endpoint "${1:-false}"
    printf 'Database schema prerequisite: the current review-job, pipeline-task, task-event, and WebSocket schemas must already be applied.\n'
}

suite_run() {
    load_api_endpoint
    load_pipeline_database_environment
    AUDIO_MODERATION_UPLOADS_BUCKET="$(deployment_output uploads_bucket_name)"
    AUDIO_MODERATION_ARTIFACTS_BUCKET="$(deployment_output artifacts_bucket_name)"
    AUDIO_MODERATION_STATE_MACHINE_ARN="$(deployment_output audio_processing_state_machine_arn)"
    export AUDIO_MODERATION_UPLOADS_BUCKET AUDIO_MODERATION_ARTIFACTS_BUCKET AUDIO_MODERATION_STATE_MACHINE_ARN
    load_task_events_endpoint
    run_deployed_cargo_test submit-audio-lambda starts_uploaded_review_evaluation_against_aws
}
