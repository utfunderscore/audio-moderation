# SocialGuard UI — Audio Moderation

A single-page React/Vite UI for the audio moderation pipeline. It takes an audio
file and advances a stage tracker through the event sequence the deployed
task-events WebSocket emits.

The UI contains **no backend interaction code**. Every backend operation is
declared in one place — `src/api/backend.ts` — and the UI depends only on that
interface. Uploads, auth, HTTP, and the WebSocket live in the implementation.

## Run

```sh
npm install
npm run dev       # http://127.0.0.1:5173
npm test          # reducer unit tests (vitest)
npm run lint
npm run build     # tsc -b && vite build
```

Point the app at a deployed stack in `.env.local`. Both values come from
Terraform's local state:

```sh
terraform -chdir=../terraform output -raw api_endpoint
terraform -chdir=../terraform output -raw pipeline_task_events_websocket_endpoint
```

- `VITE_API_ENDPOINT` — the public HTTP API base URL (`https://…/`), used for
  the Connect RPC calls.
- `VITE_TASK_EVENTS_ENDPOINT` — the deployed task-events WebSocket URL
  (`wss://…`). `ApiBackend` exchanges the evaluation's bearer token for a
  one-time ticket and sends it directly to that endpoint.

Vite expands `$VAR` references in env files, so escape the API Gateway
`$default` stage as `\$default` or it is dropped from the task-events URL.

## What it shows

A vertical timeline with one page per stage, filling in as the pipeline runs:

- **Audio** — choosing a file starts the evaluation; the playable waveform
  appears once audio processing finishes.
- **Transcription** — the transcript, read from the evaluation result.
- **Moderation** — five category scores with a flag verdict. A toast confirms
  the terminal outcome; failures render inline on the stage that failed.

Partial stage changes show `*_STARTED` as processing and `*_FINISHED` as
complete. The stage machine tolerates duplicate and out-of-order events without
regressing.

## The backend seam

`src/api/backend.ts` is the single interface for every operation the UI needs:

| Operation | Used by |
| --- | --- |
| `listJobs(userId)` | job history |
| `startEvaluation({ userId, audio })` | choosing a file |
| `subscribeTaskEvents(evaluationId, handlers)` | live stage tracking |
| `getEvaluationResult(evaluationId)` | transcript and scores |
| `getJobAudio(jobId)` | replaying a past job |

`src/main.tsx` is the composition root: it supplies the `Backend` implementation
to `<App backend={...} />`. It currently passes a placeholder that throws
`not implemented` for every operation, so the app builds and renders but performs
no backend work until a real implementation is wired in.

Task-event frames are raw event names and delivery is at-least-once, so the
reducer dedupes by name. Transcripts and scores are not carried on the stream;
they come from `getEvaluationResult` after the workflow settles.

## Layout

```
src/
  api/         Backend interface (the only backend seam)
  domain/      events, stages, moderation, jobs, pure reducer (+ tests)
  hooks/       useEvaluation, useJobHistory, useAudioInput, useElapsed, useMediaQuery
  components/
    ui/        generated shadcn/ui components
    demo/      customer-facing audio, job history, and result views
```

The event names and ordering are taken from the backend
(`backend/crates/start-evaluation-lambda`, `audio-processing-lambda`,
`transcription-caller-lambda`, `moderation-caller-lambda`,
`task-callback-lambda`, `task-event-emitter`).

## Icons and components

- shadcn/ui components are vendored under `src/components/ui/`.
- Icons come from `@untitledui/icons` (see `DEMO_DESIGN_ROUTE.md` for the mapping).
- Design notes live in `DEMO_PLAN.md` and `DEMO_DESIGN_ROUTE.md`.
