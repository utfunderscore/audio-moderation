# Project Overview

This project aims to build a reliable, asynchronous audio review platform. It accepts audio review requests, prepares and normalizes the audio, produces a transcript, evaluates the content against moderation or quality rules, persists the outcome, and exposes job status and results through an API.

## Project Goals

- Process every audio review job reliably, with explicit workflow state, bounded retries, timeouts, and durable failure records.
- Prevent duplicate jobs through idempotent request handling.
- Scale ingestion, audio processing, transcription, evaluation, and result retrieval independently.
- Preserve an auditable history of job states, model runs, errors, and model versions.
- Store large artifacts such as audio, transcripts, and detailed results in S3 while keeping queryable job and result metadata in PostgreSQL.
- Protect sensitive audio and transcript data with least-privilege access, encryption, private networking, retention controls, and sanitized logging.
- Provide end-to-end observability using a shared job/correlation ID, structured logs, metrics, traces, dashboards, and alarms.
- Support completion and failure notifications for callers and downstream systems.

## Target Architecture and Initial Scope

The intended AWS architecture uses unary Connect RPC methods defined in versioned Protobuf files for the public API. API Gateway and a Rust Lambda adapter host the initial API handlers, Step Functions provides orchestration, S3 stores artifacts, and Aurora PostgreSQL or RDS PostgreSQL stores durable state and results. Transcription and evaluation are handled by approved managed or custom model endpoints. If streaming RPCs become necessary, host the Connect service on ECS/Fargate behind an Application Load Balancer instead of extending the Lambda adapter.

The first implementation should deliver a production-shaped vertical slice: generate Rust and browser/service clients from the Protobuf contract, submit a review through Connect RPC, create its database record, run preprocessing and mocked model steps through Step Functions, persist the result, and retrieve job status through Connect RPC. Real model integrations, callbacks, notifications, alarms, and retention policies can then be added incrementally.

The demo API is unauthenticated and rate-limited only. Jobs use a configured environment `TENANT_ID` (default `default`) until authentication and per-caller tenant identity are added.

Real-time transcription, model training, a full human-review UI, and cross-region active/active operation are not first-release goals.

## Migration Policy

This project has not reached production. Update existing migration files directly when changing the current schema; do not create incremental migrations solely to preserve a deployed migration history.

## Verification Policy

Do not run `git diff --check`; it is not a useful verification step for this project.

## Deployment

Run `./deploy.sh` from the repository root to build and push the submit-review and upload-complete Lambda images, apply the Terraform configuration, and run the deployed integration test. The defaults deploy `audio-moderation` to the `dev` environment in `eu-west-2`, use tenant `default`, and read the database URL from the encrypted SSM parameter `/audio-moderation/dev/database-url`. Terraform prompts for approval; use `./deploy.sh --auto-approve` only for an unattended deployment.

The deployment requires authenticated AWS CLI access plus Cargo, Docker, Git, and Terraform. Run `./deploy.sh --help` for configuration flags and equivalent environment variables. The script generates a shared unique immutable image tag by default, bootstraps both ECR repositories when necessary, waits for both Lambda updates to complete, and prints the API endpoint. Use `--skip-integration-test` only when an infrastructure-only deployment is needed. Use the script rather than a direct first-time `terraform apply`, because both Lambda images must be pushed before Terraform can create the functions.

Run the ignored deployed integration test explicitly with `AUDIO_MODERATION_API_ENDPOINT=$(terraform -chdir=terraform output -raw api_endpoint) cargo test --package submit-audio-lambda --test deployed -- --ignored --nocapture`. It creates a real review job, uploads a small WAV object to the deployed S3 bucket, and waits for the upload-complete Lambda to queue the job; normal test runs never execute it.

See `audio_review_architecture_design_sql.md` for the detailed architecture and design decisions.
