# Database schema and review notifications

This guide explains the database in `migrations/` and the implemented flow
that lets a client subscribe to review notifications immediately after creating
a review.

It is written for developers who are new to this project. Database terms are
explained in the next section.

> **Important:** `SubmitReview` atomically creates the review, its linked
> pipeline task, inputs, step rows, and initial durable event. S3 upload
> notification only validates that existing task and starts its workflow.

## Useful terms

| Term | Plain-English meaning |
|---|---|
| Migration | A SQL file that creates or changes part of the database. |
| Row | One saved item in a table. For example, one row in `review_jobs` is one review. |
| Primary key | The field that uniquely identifies a row. |
| Foreign key | A checked link from one table to a row in another table. |
| Constraint | A database rule that prevents invalid data from being saved. |
| Index | Extra database data that makes a particular lookup faster. |
| Nullable | The field is allowed to have no value yet. SQL calls this `NULL`. |
| Tenant | The customer or workspace that owns the data. Tenant checks stop one customer from reading another customer's data. |
| Idempotent | Safe to repeat. Repeating the same request returns the original work instead of creating a duplicate. |
| Cascade delete | Deleting a parent row automatically deletes its child rows. |
| Replay | Sending saved events to a client that subscribed after those events happened. |

## Review subscription flow

The goal is for the client to create a review, subscribe, and then upload the
audio. It should not have to poll `GetReview` to discover an evaluation first.

```text
1. SubmitReview
   ├─ create the review job
   ├─ create its pipeline task at the same time
   ├─ save the expected S3 input path and empty pipeline steps
   ├─ save the EVALUATION_ACCEPTED event
   └─ return:
      • review ID
      • evaluation ID
      • review access token
      • upload URL

2. CreateReviewEventsTicket(review ID)
   ├─ check the same review access token
   ├─ find the pipeline task linked to the review
   └─ return a short-lived, one-use WebSocket ticket

3. Connect to the WebSocket
   └─ send {"action":"subscribe","ticket":"..."}

4. Upload the audio
   └─ S3 tells confirm-upload that the object now exists

5. Start processing
   ├─ confirm-upload finds the pipeline task created in step 1
   ├─ confirm-upload starts the Step Functions workflow
   └─ pipeline events are sent over the existing WebSocket subscription
```

The pipeline task can be created before the upload because its S3 location is
known in advance:

```text
s3://<uploads-bucket>/reviews/<review-id>/source
```

Creating a pipeline task does **not** start processing. The S3 upload event is
still required before the workflow can start.

### Why there are both access tokens and tickets

The client generates a `review_v1` access token before calling `SubmitReview`:
`review_v1.` followed by base64url (without padding) for 32 cryptographically
random bytes. It sends that token in `Authorization: Bearer ...`, retains it as
a sensitive bearer credential, and uses the same token for the review, its
linked evaluation, and creating an event ticket. The service never returns or
reissues it.

The WebSocket ticket has a smaller job. It is random, expires quickly, and can
only be used once. This avoids sending the longer-lived review token through
the WebSocket connection. Creating a ticket does not give the caller access to
a different review.

## How the tables connect

```text
review_jobs
    └── pipeline_tasks
          ├── pipeline_task_inputs
          ├── audio_processing_tasks
          ├── transcription_tasks
          ├── moderation_tasks
          ├── pipeline_callback_attempts
          ├── pipeline_task_events
          ├── pipeline_task_event_tickets
          └── pipeline_task_websocket_connections
```

A review can have at most one pipeline task. A task created directly through
`StartEvaluation` does not have a review, so its `review_job_id` is empty.

### Keeping direct tasks and review tasks separate

There are two ways to create a pipeline task, and they deliberately have
different responsibilities:

| Creation route | What it creates | Can it link a review? |
|---|---|---|
| Direct evaluation (`StartEvaluation`) | An ordinary task with caller-supplied audio object URIs. | No. It always saves an empty (`NULL`) `review_job_id`. |
| Review submission (`SubmitReview`) | A review and its one expected task in the same database transaction. | Yes. This is the only application path that creates the review-to-task link. |

The ordinary database API is `NewPipelineTask` with
`PipelineTaskStore::create_or_get`. `NewPipelineTask` intentionally does not
contain a `review_job_id` field, so callers cannot accidentally turn a direct
task into a review task. A `caller_reference` that happens to look like
`review-job:42` is only text for tracking; it does not create a relationship.

The review-specific API is `NewReviewPipelineTask` with
`PipelineTaskStore::create_or_get_review`. It creates or replays the review and
its task atomically. On a replay, it checks that the established task still has
the expected review ID, tenant, derived idempotency key, caller reference, and
S3 input URI. If those values do not match, the store returns a typed
`ReviewTaskConflict` error instead of exposing a raw database unique-constraint
error. This makes a broken or legacy mapping visible to application code without
silently attaching the wrong task.

This separation is important even though the database also has foreign-key and
unique rules. The application API prevents ordinary callers from requesting an
invalid link in the first place, while the review-specific transaction owns the
one-to-one link and verifies it when reused.

## Migration 1: review jobs

File: `migrations/1_create_review_jobs.sql`

### Review statuses

| Status | Meaning |
|---|---|
| `AWAITING_UPLOAD` | The review exists, but the upload has not been confirmed. |
| `PENDING_PROCESSING` | The upload exists and is waiting to be dispatched. |
| `PROCESSING` | The processing workflow has started. |
| `COMPLETED` | Processing finished successfully. |
| `ERROR` | Dispatch or processing failed. |

### `review_jobs` fields

| Field | What it stores and why |
|---|---|
| `job_id` | The database ID for the review. PostgreSQL assigns the next number automatically. The API currently returns this as `review_id`. |
| `tenant_id` | The customer or workspace that owns the review. Every review lookup must also check this field. |
| `idempotency_key` | A UUID supplied only for retry/deduplication. The same tenant and key returns an existing review only when the caller also presents its matching review token. |
| `access_token_hash` | SHA-256 hex digest of the client-generated review token. The raw token is never stored. |
| `status` | The current review status. New reviews start as `AWAITING_UPLOAD`. |
| `input_file_path` | The expected S3 object key. PostgreSQL creates it automatically as `reviews/<job_id>/source`; application code cannot choose a different value. |
| `created_at` | When the review was created. Opaque review access lasts while this review exists. |
| `updated_at` | When the review status was last changed. |

### Rules and lookup helpers

| Rule or index | Why it exists |
|---|---|
| Primary key on `job_id` | Makes every review ID unique. |
| `review_jobs_idempotency_unique` | Stops one tenant from creating two reviews with the same idempotency key. Different tenants may use the same key. |
| `review_jobs_job_tenant_unique` | Lets the task link to both the review ID and its tenant. This prevents a task from linking to another tenant's review. |
| `idx_review_jobs_tenant_created` | Makes it faster to list one tenant's newest reviews first. |

The raw `review_v1` token is **not** saved in this table. Only its SHA-256
digest is stored, so an idempotency key alone cannot retrieve a review or issue
an upload URL.

## Migration 2: pipeline work

File: `migrations/2_create_pipeline_tasks.sql`

### Final pipeline outcomes

| Outcome | Meaning |
|---|---|
| `SUCCEEDED` | All required processing finished successfully. |
| `FAILED` | Dispatch or a processing step failed permanently. |
| `TIMED_OUT` | Processing took longer than allowed. |
| `CANCELLED` | Processing was intentionally stopped. |

An empty outcome means the task has not finished yet.

### Individual step statuses

| Status | Meaning |
|---|---|
| `PENDING` | The step has not started. |
| `PROCESSING` | The step is running or waiting for an external service to call back. |
| `COMPLETED` | The step finished successfully and saved its output. |
| `FAILED` | The step failed. |

### Callback step names

| Step | Meaning |
|---|---|
| `TRANSCRIPTION` | The callback belongs to the transcription step. |
| `MODERATION` | The callback belongs to the moderation step. |

### `pipeline_tasks` fields

| Field | What it stores and why |
|---|---|
| `task_id` | The internal database ID for a pipeline task. Step rows, events, tickets, and WebSocket subscriptions use this ID. |
| `evaluation_id` | The public UUID returned for an evaluation. A UUID is a long random-looking ID that is safe to expose. |
| `tenant_id` | The customer or workspace that owns the task. It must match the linked review's tenant. |
| `review_job_id` | The review that created this task. It is empty for direct evaluations. Only the review-specific creation path may set it. `UNIQUE` means one review cannot have two tasks. |
| `idempotency_key` | The key that prevents duplicate tasks. Direct callers provide their own key. The review-specific path derives a stable key such as `review-upload:<review-id>`. |
| `caller_reference` | Optional text supplied for tracking. Reviews use a derived value such as `review-job:<review-id>`. This is only a label; code should use `review_job_id` for the real relationship. |
| `outcome` | The final result. It is empty until the task finishes. |
| `execution_arn` | The AWS Step Functions execution ID after dispatch. Saving it helps prevent starting the same workflow twice. |
| `dispatch_started_at` | When the current dispatch attempt claimed the task. Other requests use this as a temporary lock so they do not dispatch the task at the same time. |
| `attempt_count` | How many times code has claimed the task for dispatch. It starts at zero and cannot be negative. |
| `completed_at` | When the task finished. It must be filled in exactly when `outcome` is filled in. |
| `created_at` | When the task was created. In the target flow, a review task is created during `SubmitReview`, before upload. |
| `updated_at` | When task-level data last changed. |

### Rules and lookup helpers

| Rule or index | Why it exists |
|---|---|
| `pipeline_tasks_outcome_completion_consistent` | Prevents a task from having an outcome without a completion time, or a completion time without an outcome. |
| `pipeline_tasks_evaluation_id_unique` | Stops two tasks from sharing one public evaluation ID. |
| `pipeline_tasks_idempotency_unique` | Stops one tenant from creating two tasks with the same idempotency key. |
| Unique rule on `review_job_id` | Stops one review from owning more than one pipeline task. Multiple direct evaluations are still allowed because their value is empty. |
| `pipeline_tasks_review_job_tenant_fk` | Requires the linked review to exist under the same tenant. The review cannot be deleted while its task still exists. |
| `idx_pipeline_tasks_outcome` | Makes searches by final outcome faster. |

### `pipeline_task_inputs` fields

| Field | What it stores and why |
|---|---|
| `task_id` | The pipeline task that owns the input. Deleting the task also deletes its inputs. |
| `sequence` | The input's position, starting at zero. This preserves order when several audio files must be joined. It cannot be negative. |
| `audio_s3_uri` | The full S3 address of the audio. For reviews, it can be saved before upload because the address is known in advance. |

`task_id` and `sequence` together must be unique, so a task can only have one
input at each position.

### `audio_processing_tasks` fields

| Field | What it stores and why |
|---|---|
| `task_id` | The parent pipeline task. It is also this row's unique ID, so each pipeline has one audio-processing row. |
| `status` | The step status. It starts as `PENDING`. |
| `stitched_audio_s3_uri` | The S3 address of the converted or combined audio. It is required when the step is `COMPLETED`. |
| `error_code` | Optional short code that application code can use to identify a failure. |
| `error_message` | Optional readable failure description. It must not contain secrets or unnecessary sensitive data. |
| `started_at` | When audio processing began. |
| `completed_at` | When the step finished. It cannot be earlier than `started_at`. |
| `updated_at` | When this step was last changed. |

### `transcription_tasks` fields

| Field | What it stores and why |
|---|---|
| `task_id` | The parent pipeline task. Each pipeline has one transcription row. |
| `status` | The step status. It starts as `PENDING`. |
| `external_task_id` | The transcription service's job ID. It must be unique so one external job cannot belong to two pipeline tasks. |
| `transcript` | The text produced from the audio. It is required when the step is `COMPLETED`. |
| `error_code` | Optional short failure code. |
| `error_message` | Optional readable failure description. |
| `started_at` | When transcription began. |
| `completed_at` | When the step finished. It cannot be earlier than `started_at`. |
| `updated_at` | When this step was last changed. |

### `pipeline_callback_attempts` fields

External transcription and moderation jobs call the backend when they finish.
AWS gives each waiting workflow step a secret callback token.

| Field | What it stores and why |
|---|---|
| `task_id` | The pipeline task waiting for the callback. |
| `step` | Whether the callback is for transcription or moderation. |
| `task_token_hash` | A one-way fingerprint of the secret AWS callback token. The real token is not saved. It must be unique so the callback resolves to exactly one attempt. |
| `created_at` | When this callback attempt was registered. |

The combination of task, step, and token fingerprint must be unique. Old
attempts are kept so retries can still be matched safely.

### `moderation_tasks` fields

| Field | What it stores and why |
|---|---|
| `task_id` | The parent pipeline task. Each pipeline has one moderation row. |
| `status` | The step status. It starts as `PENDING`. |
| `external_task_id` | The moderation service's job ID. It must be unique. |
| `sexual` | Sexual-content score from 0 to 1. |
| `hate_or_discrimination` | Hate or discrimination score from 0 to 1. |
| `harassment_or_abuse` | Harassment or abuse score from 0 to 1. |
| `violence_or_threats` | Violence or threats score from 0 to 1. |
| `asking_for_pii` | Score for asking someone to provide personal information, from 0 to 1. |
| `error_code` | Optional short failure code. |
| `error_message` | Optional readable failure description. |
| `started_at` | When moderation began. |
| `completed_at` | When the step finished. It cannot be earlier than `started_at`. |
| `updated_at` | When this step was last changed. |

When moderation is `COMPLETED`, all five scores must be present. Every saved
score must be between 0 and 1.

## Migration 3: saved task events

File: `migrations/3_create_pipeline_task_events.sql`

### `pipeline_task_events` fields

| Field | What it stores and why |
|---|---|
| `event_id` | An automatically increasing event ID. The server uses it to replay events in saved order. |
| `task_id` | The task this event belongs to. Deleting the task also deletes its events. |
| `event_name` | The event text sent to clients, such as `EVALUATION_ACCEPTED` or `ASR_STARTED`. It cannot be empty. |
| `created_at` | When the event was first saved. |

One task cannot save the same event name twice. The
`idx_pipeline_task_events_task_id_event_id` index makes it faster to load one
task's events in order.

Clients should still handle duplicate messages. A client can receive an event
once from saved-event replay and again from a live notification if both happen
at nearly the same time.

In the implemented review flow, `EVALUATION_ACCEPTED` is saved when the review and task are
created. A client that subscribes just after `SubmitReview` receives it through
replay.

## Migration 4: WebSocket subscriptions

File: `migrations/4_create_pipeline_task_websocket_connections.sql`

### `pipeline_task_websocket_connections` fields

| Field | What it stores and why |
|---|---|
| `task_id` | The task whose events should be sent to this connection. Deleting the task removes the subscription. |
| `connection_id` | The ID API Gateway gives to an open WebSocket connection. It cannot be empty. |
| `created_at` | When the connection subscribed to the task. |

Each connection can subscribe to exactly one task stream; reusing it for a
different review/task is rejected, so a client must open a separate socket. The
`idx_pipeline_task_websocket_connections_connection_id` index makes it faster
to remove subscriptions when a socket disconnects or is found to be closed.

### `pipeline_task_event_tickets` fields

| Field | What it stores and why |
|---|---|
| `ticket_hash` | A one-way fingerprint of the random ticket. The real ticket is returned to the client but is not saved. |
| `task_id` | The one task stream this ticket allows the client to subscribe to. |
| `expires_at` | The time after which the ticket can no longer be used. |
| `created_at` | When the ticket was created. |

The expiry must be later than the creation time. The
`idx_pipeline_task_event_tickets_expiry` index makes expired-ticket cleanup
faster.

When a client subscribes, the server finds and deletes the ticket in one
database operation. This makes the ticket one-use, even if two subscribe
requests arrive at almost the same time.

## IDs and secrets at a glance

These values have different jobs and should not be treated as interchangeable:

| Value | Purpose |
|---|---|
| Review ID | Identifies the upload-first review from submission through completion. |
| Evaluation ID | Public ID for the linked processing pipeline. |
| Task ID | Internal database ID used to connect pipeline tables. It is not a secret. |
| `review_v1` token | Client-generated opaque bearer credential for one review and its linked evaluation. Only its SHA-256 digest is stored. |
| `eval_v1` token | Proves access to an evaluation created directly through `StartEvaluation`. It is signed but not stored in the database. |
| `wst_v1` ticket | Short-lived, one-use WebSocket subscription ticket. Only its fingerprint is stored. |
| AWS callback token | Secret used by an external processing job to resume Step Functions. Only its fingerprint is stored. |

For the implemented review flow, the same `review_v1` token authorizes `GetReview`,
the linked `GetEvaluation`, `CreateReviewEventsTicket`, and linked
`CreateTaskEventsTicket`. The client can
create a ticket and subscribe as soon as `SubmitReview` returns; it does not
need to poll for an evaluation ID.
