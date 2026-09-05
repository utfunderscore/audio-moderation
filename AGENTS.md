## Migration Policy

This project has not reached production. Update existing migration files directly when changing the current schema; do not create incremental migrations solely to preserve a deployed migration history.

## Verification Policy

Do not run `git diff --check`; it is not a useful verification step for this project.

## Deployment

All agents must use the local `admin` AWS CLI profile for AWS CLI, Terraform, and deployment commands. Prefix commands with `AWS_PROFILE=admin`; do not use the default profile or another AWS account.

Run `AWS_PROFILE=admin ./deploy-upload-flow.sh` from the repository root to deploy the review-submission, file-upload, and audio-conversion slice: it builds and pushes the submit-audio, confirm-upload, and audio-processing Lambda images; applies their API Gateway, S3, and Step Functions Terraform configuration; and runs the deployed upload-flow integration test. It does not deploy or test moderation ingress, transcription, or moderation evaluation. The defaults deploy `audio-moderation` to the `dev` environment in `eu-west-2`, use tenant `default`, and read the database URL from the encrypted SSM parameter `/audio-moderation/dev/database-url`. Terraform prompts for approval; use `AWS_PROFILE=admin ./deploy-upload-flow.sh --auto-approve` only for an unattended deployment.

The upload-flow deployment requires authenticated AWS CLI access through the `admin` profile plus Cargo, Docker, Git, and Terraform. Run `AWS_PROFILE=admin ./deploy-upload-flow.sh --help` for configuration flags and equivalent environment variables. The script generates a shared unique immutable image tag by default, bootstraps all three ECR repositories when necessary, waits for all three Lambda updates to complete, and prints the API endpoint. Use `--skip-integration-test` only when an infrastructure-only deployment is needed. Use the script rather than a direct first-time `terraform apply`, because all Lambda images must be pushed before Terraform can create the functions.

Run the currently implemented ignored upload-flow deployment test explicitly with `AWS_PROFILE=admin AUDIO_MODERATION_API_ENDPOINT=$(AWS_PROFILE=admin terraform -chdir=terraform output -raw api_endpoint) cargo test --package submit-audio-lambda --test deployed -- --ignored --nocapture`. It exercises the existing single-file upload slice; normal test runs never execute it. Update this test alongside implementation of the separate review-request and evaluation-job flows.
