#!/usr/bin/env bash

set -Eeuo pipefail
umask 077
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/aws-context.sh"
source "${SCRIPT_DIR}/lib/deployment-outputs.sh"
source "${SCRIPT_DIR}/deployed-tests/environment.sh"

usage() {
    cat <<'EOF'
Usage:
  AWS_PROFILE=admin ./scripts/test-deployed.sh preflight <suite> [options]
  AWS_PROFILE=admin ./scripts/test-deployed.sh test <suite> [options]

Tests an existing deployment; never applies infrastructure or publishes images.
test runs suite-specific preflight automatically. preflight does not decrypt
secrets or create fixtures.

Suites:
  review-submit        SubmitReview, evaluation reads, and task-event tickets
  review-confirmation Upload-triggered dispatch (not full model completion)
  audio-conversion    Synchronous audio-processing Lambda invocation
  task-events         WebSocket replay, live delivery, and disconnect cleanup

task-callback, transcription-caller, and moderation-caller remain unsupported:
they need real persisted callback/task-token fixtures. No placeholder tokens.

Options (also configurable through corresponding uppercase environment variables):
  --region REGION                        Default: eu-west-2
  --project-name NAME                    Default: audio-moderation
  --environment NAME                     Default: dev
  --tenant-id ID                         Audio-conversion tenant (default: default)
  --database-parameter-name NAME         SecureString database URL parameter
  --turnstile-secret-parameter-name NAME  Deployed Turnstile secret parameter
  --audio-file FILE                      Required for audio-conversion
  -h, --help

Review suites require a fresh TURNSTILE_TEST_TOKEN. Endpoint overrides use
AUDIO_MODERATION_API_ENDPOINT and AUDIO_MODERATION_TASK_EVENTS_ENDPOINT.
See scripts/deployed-tests/README.md for prerequisites and fixture cleanup.
EOF
}

ACTION="${1:-}"
case "${ACTION}" in
    ''|-h|--help) usage; exit 0 ;;
    preflight|test) shift ;;
    *) printf 'Unknown deployed-test command: %s\n' "${ACTION}" >&2; usage >&2; exit 2 ;;
esac
if [[ "${1:-}" == -h || "${1:-}" == --help ]]; then usage; exit 0; fi
require_value "${ACTION}" "${1:-}"
SUITE="$1"
shift
case "${SUITE}" in
    review-submit|review-confirmation|audio-conversion|task-events) ;;
    task-callback|transcription-caller|moderation-caller)
        printf '%s is unsupported: real persisted task/callback attempts and Step Functions task-token fixtures are required. Do not use placeholder tokens.\n' "${SUITE}" >&2
        exit 1
        ;;
    *) printf 'Unknown suite: %s\n' "${SUITE}" >&2; usage >&2; exit 2 ;;
esac
parse_test_options "$@"
configure_aws_context
configure_test_environment
source "${SCRIPT_DIR}/deployed-tests/preflight.sh"
# Source only the selected suite, never deployment code or unrelated fixtures.
source "${SCRIPT_DIR}/deployed-tests/suites/${SUITE}.sh"

case "${ACTION}" in
    preflight) test_preflight "${TEST_ALLOW_MISSING_DEPLOYED_ENDPOINT:-false}" ;;
    test) test_preflight false; suite_run ;;
esac
