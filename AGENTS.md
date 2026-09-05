# Project Overview

This repository contains two independent systems and their optional integration. The review submission system owns review requests, expected file uploads, and a coarse request status. It does not own moderation workflow state. The audio moderation system independently accepts one or more existing audio object references, prepares and normalizes the audio, produces a transcript, evaluates the content against moderation or quality rules, persists its own state and outcome, and publishes completion or failure events. When combined into the full flow, the review submission system is one client of the moderation system; other authorized services can invoke moderation without creating a review request.

## Project Goals

- Process every review request and evaluation job reliably, with explicit independently owned state, bounded retries, timeouts, and durable failure records.
- Prevent duplicate review requests and evaluation jobs through separately scoped idempotent request handling.
- Scale and deploy ingestion, moderation, and result retrieval independently.
- Allow authorized services to invoke moderation directly with ordered sets of existing audio objects.
- Preserve an auditable history of job states, model runs, errors, and model versions.
- Store large artifacts such as audio, transcripts, and detailed results in S3 while keeping queryable job and result metadata in PostgreSQL.
- Protect sensitive audio and transcript data with least-privilege access, encryption, private networking, retention controls, and sanitized logging.
- Provide end-to-end observability using distinct review, evaluation, execution-attempt, and caller-correlation identifiers, plus structured logs, metrics, traces, dashboards, and alarms.
- Support completion and failure notifications for callers and downstream systems.

## Target Architecture and Initial Scope

The review submission system uses unary Connect RPC methods through API Gateway and a Rust Lambda adapter. It creates review requests, manages expected uploads in S3, persists its own state in PostgreSQL, and exposes only `AWAITING_UPLOAD`, `PROCESSING`, `COMPLETE`, and `ERROR`. Its downstream processing dependency is an interface expressed in object references, an opaque processing reference, and terminal outcome events.

The audio moderation system has its own unary Connect RPC ingress, persistence ownership, and deployment lifecycle. Its ingress accepts ordered existing audio object references, creates an evaluation job, and durably dispatches Step Functions. Callers do not invoke Step Functions directly. The system owns preprocessing, transcription, evaluation, detailed state, execution attempts, results, and completion/failure events. If streaming RPCs become necessary, host this Connect service on ECS/Fargate behind an Application Load Balancer instead of extending the Lambda adapter.

For the full flow, an adapter connects the review system's downstream processing interface to the moderation API and events. The review system stores `evaluationId` only as an opaque processing reference, while moderation stores `reviewId` only as an opaque caller reference. Neither system accesses the other's tables.

The first implementation should prove both systems independently, then their integration. Test review submission through upload and a mocked downstream processor. Test moderation directly with multiple ordered file references and no review request. Finally connect the review processing adapter to moderation, consume its terminal event, and expose the projected coarse review status. Real model integrations, callbacks, notifications, alarms, and retention policies can then be added incrementally.

The demo API is unauthenticated and rate-limited only. Jobs use a configured environment `TENANT_ID` (default `default`) until authentication and per-caller tenant identity are added.

Real-time transcription, model training, a full human-review UI, and cross-region active/active operation are not first-release goals.

## Migration Policy

This project has not reached production. Update existing migration files directly when changing the current schema; do not create incremental migrations solely to preserve a deployed migration history.

## Verification Policy

Do not run `git diff --check`; it is not a useful verification step for this project.

## Deployment

Run `./deploy-upload-flow.sh` from the repository root to deploy the review-submission and file-upload slice: it builds and pushes the submit-audio and confirm-upload Lambda images, applies their API Gateway and S3 Terraform configuration, and runs the deployed upload-flow integration test. It does not deploy or test audio processing, transcription, or moderation evaluation. The defaults deploy `audio-moderation` to the `dev` environment in `eu-west-2`, use tenant `default`, and read the database URL from the encrypted SSM parameter `/audio-moderation/dev/database-url`. Terraform prompts for approval; use `./deploy-upload-flow.sh --auto-approve` only for an unattended deployment.

The upload-flow deployment requires authenticated AWS CLI access plus Cargo, Docker, Git, and Terraform. Run `./deploy-upload-flow.sh --help` for configuration flags and equivalent environment variables. The script generates a shared unique immutable image tag by default, bootstraps both ECR repositories when necessary, waits for both Lambda updates to complete, and prints the API endpoint. Use `--skip-integration-test` only when an infrastructure-only deployment is needed. Use the script rather than a direct first-time `terraform apply`, because both Lambda images must be pushed before Terraform can create the functions.

Run the currently implemented ignored upload-flow deployment test explicitly with `AUDIO_MODERATION_API_ENDPOINT=$(terraform -chdir=terraform output -raw api_endpoint) cargo test --package submit-audio-lambda --test deployed -- --ignored --nocapture`. It exercises the existing single-file upload slice; normal test runs never execute it. Update this test alongside implementation of the separate review-request and evaluation-job flows.
