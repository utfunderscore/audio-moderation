#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
export AWS_PROFILE=admin

exec "${ROOT_DIR}/deploy-transcription-caller.sh" "$@"
