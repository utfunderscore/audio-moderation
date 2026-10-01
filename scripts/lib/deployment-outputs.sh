#!/usr/bin/env bash

deployment_output() {
    AWS_PROFILE=admin terraform -chdir="${TERRAFORM_DIR}" output -raw "$1"
}
