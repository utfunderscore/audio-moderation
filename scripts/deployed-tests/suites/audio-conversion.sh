#!/usr/bin/env bash

suite_preflight() {
    require_command cargo
    validate_secure_parameter "${DATABASE_URL_PARAMETER}"
    printf 'Database schema prerequisite: the current pipeline-task and step schemas must already be applied.\n'
}

suite_run() {
    load_pipeline_database_environment
    AUDIO_MODERATION_UPLOADS_BUCKET="$(deployment_output uploads_bucket_name)"
    AUDIO_MODERATION_ARTIFACTS_BUCKET="$(deployment_output artifacts_bucket_name)"
    AUDIO_MODERATION_AUDIO_PROCESSING_FUNCTION_NAME="$(deployment_output audio_processing_function_name)"
    AUDIO_MODERATION_AUDIO_FILE="${AUDIO_FILE}"
    AUDIO_MODERATION_TENANT_ID="${TENANT_ID}"
    export AUDIO_MODERATION_UPLOADS_BUCKET AUDIO_MODERATION_ARTIFACTS_BUCKET
    export AUDIO_MODERATION_AUDIO_PROCESSING_FUNCTION_NAME AUDIO_MODERATION_AUDIO_FILE AUDIO_MODERATION_TENANT_ID
    run_deployed_cargo_test audio-processing-lambda converts_audio_against_aws
}
