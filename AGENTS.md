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

The intended AWS architecture uses API Gateway and Lambda for API handling, Step Functions for orchestration, S3 for artifacts, and Aurora PostgreSQL or RDS PostgreSQL for durable state and results. Transcription and evaluation are handled by approved managed or custom model endpoints.

The first implementation should deliver a production-shaped vertical slice: submit a review, create its database record, run preprocessing and mocked model steps through Step Functions, persist the result, and retrieve job status. Real model integrations, callbacks, notifications, alarms, and retention policies can then be added incrementally.

Real-time transcription, model training, a full human-review UI, and cross-region active/active operation are not first-release goals.

## Migration Policy

This project has not reached production. Update existing migration files directly when changing the current schema; do not create incremental migrations solely to preserve a deployed migration history.

## Verification Policy

Do not run `git diff --check`; it is not a useful verification step for this project.

See `audio_review_architecture_design_sql.md` for the detailed architecture and design decisions.
