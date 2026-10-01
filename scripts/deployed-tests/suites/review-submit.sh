#!/usr/bin/env bash

suite_preflight() {
    require_command cargo
    validate_secure_parameter "${DATABASE_URL_PARAMETER}"
    validate_secure_parameter "${TURNSTILE_SECRET_KEY_PARAMETER}"
    printf 'Database schema prerequisite: the review-job schema must already be current.\n'
}

suite_run() {
    load_api_endpoint
    run_deployed_cargo_test submit-audio-lambda submits_review_against_aws
}
