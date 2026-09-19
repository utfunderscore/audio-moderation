# Accountless Demo Abuse Protection Plan

## Status

Evaluation capability access and one-time WebSocket tickets are implemented as
described in [Implemented access controls](#implemented-access-controls). Anonymous
submission controls, retention, monitoring, and launch limits remain deferred and
must be completed before public launch.

## Context

The demo will allow users to submit audio and retrieve pipeline results without an
account or login. Evaluation-scoped capability tokens can protect result access, but
they do not prevent anonymous users from creating excessive work and incurring S3,
Lambda, Step Functions, transcription, moderation, or database costs.

## Goals

- Limit automated and high-volume task creation.
- Bound the cost and resource use of every accepted evaluation.
- Protect evaluation results with an unguessable, evaluation-scoped capability.
- Detect abuse and provide an operational kill switch.
- Avoid collecting unnecessary identifying information.

## Implemented access controls

### Evaluation capability tokens

- An evaluation ID is an unguessable public identifier. Its access token is a
  deterministic HMAC-SHA-256 value derived from that ID and the global evaluation
  access secret; the token is prefixed `eval_v1.`.
- `StartEvaluation` returns the same token for an idempotent replay as for the
  original request. Tokens are not stored per evaluation, and no per-evaluation
  token digest or first-issuance record exists.
- `GetEvaluation` and `CreateTaskEventsTicket` require
  `Authorization: Bearer <token>`. Verification binds the token to the requested
  evaluation ID.
- Rotating the global evaluation access secret is the current revocation mechanism:
  it invalidates all previously issued evaluation tokens. Individual-token
  revocation is not implemented.
- Review-triggered evaluations do not expose an evaluation access token through the
  review API. `SubmitReview` returns review/upload details only, not an evaluation
  ID or access token.
- Do not place access tokens in URLs, logs, events, analytics, or WebSocket frames.
- `CreateTaskEventsTicket` mints a random, task-scoped ticket after capability
  authorization. The server stores only its SHA-256 hash, expires it after two
  minutes, and consumes it once. Clients send the ticket in the WebSocket
  subscription body instead of placing the long-lived token in the WebSocket URL.

## Deferred controls

### Limit anonymous submission

- Apply API Gateway or AWS WAF rate limits to submission and evaluation-start
  endpoints, with stricter burst limits than sustained limits.
- Add a browser challenge such as Turnstile when the demo is made public. Verify
  challenge responses server-side and prevent response replay.
- Limit concurrent active evaluations globally and per anonymous client signal.
- Define a daily submission or spend ceiling and reject new work when it is reached.
- Do not rely on CORS, evaluation IDs, or client-supplied headers as abuse controls.

### Bound work per evaluation

- Restrict upload size, supported media types, object count, and total audio
  duration before starting expensive processing.
- Configure hard timeouts and retry limits for every pipeline step.
- Prevent an idempotency key from creating duplicate executions.
- Reject inputs that are outside the documented demo limits before model calls.

### Limit retention and storage cost

- Expire uploaded and generated audio after a short documented retention period.
- Expire evaluation records, transcripts, moderation results, events, and access
  capabilities on the same schedule unless operational data requires less time.
- Remove abandoned uploads and pipeline artifacts for failed or timed-out tasks.

### Monitor and respond

- Track accepted, rejected, throttled, failed, and concurrently active evaluations.
- Add AWS budget and service-usage alarms with actionable thresholds.
- Alert on unusual submission rates, repeated invalid media, and challenge failures.
- Provide a configuration-based kill switch that disables new anonymous submissions
  without preventing access to already-created evaluations.
- Keep logs free of transcripts, capability tokens, presigned URLs, and unnecessary
  client identifiers.

## Decisions required before public launch

- Public launch limits: requests per minute, concurrent tasks, upload bytes, audio
  duration, object count, and daily spend.
- Challenge provider and the conditions under which a challenge is required.
- Evaluation and artifact retention periods.
- Whether browser access should survive a tab close (`localStorage`) or only a page
  refresh in the current tab (`sessionStorage`).

## Minimum launch criteria

- Unauthorized and cross-evaluation result requests are rejected.
- Submission throttling and per-evaluation input limits are tested.
- Duplicate requests cannot create duplicate paid pipeline executions.
- Budget alarms and the anonymous-submission kill switch are tested.
- Lifecycle cleanup is enabled for audio artifacts and persisted evaluation data.
- Security tests confirm that tokens do not appear in application logs or URLs.
