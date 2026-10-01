#!/usr/bin/env bash

suite_preflight() {
    require_command cargo
    validate_secure_parameter "${DATABASE_URL_PARAMETER}"
    validate_task_events_endpoint "${1:-false}"
    printf 'Database schema prerequisite: the pipeline-task event and WebSocket connection schemas must already be current.\n'
}

suite_run() {
    load_pipeline_database_environment
    load_task_events_endpoint
    run_deployed_cargo_test task-events-lambda replays_live_delivers_and_cleans_up_task_events
}
