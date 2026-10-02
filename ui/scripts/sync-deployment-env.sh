#!/usr/bin/env bash

# After an approved deployment, copy only public Terraform outputs to the UI.
set -Eeuo pipefail
umask 077

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
UI_ENV_DIR="${UI_ENV_DIR:-${ROOT_DIR}/ui}"
TERRAFORM_DIR="${TERRAFORM_DIR:-${ROOT_DIR}/terraform}"

sitekey="$(AWS_PROFILE=admin terraform -chdir="${TERRAFORM_DIR}" output -raw turnstile_sitekey)"
api_endpoint="$(AWS_PROFILE=admin terraform -chdir="${TERRAFORM_DIR}" output -raw api_endpoint)"
events_endpoint="$(AWS_PROFILE=admin terraform -chdir="${TERRAFORM_DIR}" output -raw pipeline_task_events_websocket_endpoint)"

if [[ ! "${sitekey}" =~ ^[a-zA-Z0-9_-]{1,64}$ || ! "${api_endpoint}" =~ ^https://[^[:space:]]+$ || ! "${events_endpoint}" =~ ^wss://[^[:space:]]+$ ]]; then
    printf 'Refusing to write invalid public Terraform outputs to UI env files.\n' >&2
    exit 1
fi

# Vite expands $NAME in env files. Preserve the literal $default API Gateway stage.
escape_vite_dollars() { sed 's/\$/\\$/g'; }
api_endpoint="$(printf '%s' "${api_endpoint}" | escape_vite_dollars)"
events_endpoint="$(printf '%s' "${events_endpoint}" | escape_vite_dollars)"

for filename in .env.local .env.production.local; do
    file="${UI_ENV_DIR}/${filename}"
    temp="$(mktemp "${file}.tmp.XXXXXX")"
    api_present=false
    events_present=false
    {
        if [[ -f "${file}" ]]; then
            while IFS= read -r line || [[ -n "${line}" ]]; do
                case "${line}" in
                    VITE_TURNSTILE_SITE_KEY=*|VITE_TURNSTILE_ACTION=*) continue ;;
                    VITE_API_ENDPOINT=*) api_present=true ;;
                    VITE_TASK_EVENTS_ENDPOINT=*) events_present=true ;;
                esac
                printf '%s\n' "${line}"
            done < "${file}"
        fi
        if [[ "${api_present}" == false ]]; then printf 'VITE_API_ENDPOINT=%s\n' "${api_endpoint}"; fi
        if [[ "${events_present}" == false ]]; then printf 'VITE_TASK_EVENTS_ENDPOINT=%s\n' "${events_endpoint}"; fi
        printf 'VITE_TURNSTILE_SITE_KEY=%s\n' "${sitekey}"
    } > "${temp}"
    mv -- "${temp}" "${file}"
    printf 'Updated public UI configuration: ui/%s\n' "${filename}"
done
