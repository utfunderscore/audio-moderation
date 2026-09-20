# SocialGuard UI — Audio Moderation Demo

A single-page demonstration of the audio moderation pipeline. It takes an audio
file and advances a stage tracker through the same event sequence the deployed
task-events WebSocket emits.

**Everything is simulated in the browser.** There is no HTTP call, S3 upload, or
WebSocket connection. The scripts in this app mirror the real protocol so the
mock can be swapped for the real transport later (see below).

## Run

```sh
npm install
npm run dev       # http://localhost:5173
npm test          # reducer unit tests (vitest)
npm run lint
npm run build     # tsc -b && vite build
```

To expose the dev server publicly over Tailscale Funnel, see
[TAILSCALE_FUNNEL.md](TAILSCALE_FUNNEL.md).

## What it shows

A vertical timeline with one page per stage, filling in as the pipeline runs:

- **Audio** — choosing a file starts the run immediately; the playable waveform
  and source URI appear only once audio processing finishes. The source URI
  becomes the stitched `evaluations/<id>.wav`.
- **Transcription** — the transcript, 30 words from the sample.
- **Moderation** — five category scores with a flag verdict. This is where the
  timeline ends; the toast confirms the outcome.

The failure states render inline on the stage that failed.

Partial stage changes show `*_STARTED` as processing and `*_FINISHED` as
complete. The mock also covers replay-then-live delivery, at-least-once
duplicates, and unknown future events. Transcript and score values are scenario
fixtures: the real WebSocket carries event names only.

Technical test-harness settings are intentionally kept out of the product UI.

The event names and ordering are taken from the backend
(`backend/crates/start-evaluation-lambda`, `audio-processing-lambda`,
`transcription-caller-lambda`, `moderation-caller-lambda`,
`task-callback-lambda`, `task-event-emitter`).

## Layout

```
src/
  domain/      events, stage definitions, pure reducer (+ tests)
  transport/   TaskEventsClient seam, mock client, scenarios, real WS stub
  hooks/       usePipelineRun, useAudioInput, useElapsed
  components/
    ui/        generated shadcn/ui components
    demo/      customer-facing audio, job history, and result views
```

## Swapping in the real transport

`src/transport/webSocketClient.ts` implements `TaskEventsClient` against the
real `wss://` endpoint shape, but it is not usable against the deployed server:
it is unwired and still sends `{"action":"subscribe","taskId":<number>}`.
The deployed review handshake is to retain the short-lived review access token from
`SubmitReview`, poll `GetReview` until it includes an evaluation ID, and use that
same bearer token with `GetEvaluation` and `CreateTaskEventsTicket`. The latter
returns a one-time ticket, then the client sends
`{"action":"subscribe","ticket":<ticket>}`. Frames are raw UTF-8 event names
with replay-then-live, at-least-once delivery. Updating the adapter and wiring it
up require an API endpoint, authorization/token exchange, and CORS configuration;
all remain deliberately out of scope for this UI-only phase.

## Icons and components

- shadcn/ui components are vendored under `src/components/ui/`.
- Icons come from `@untitledui/icons` (see `DEMO_DESIGN_ROUTE.md` for the mapping).
- Design notes live in `DEMO_PLAN.md` and `DEMO_DESIGN_ROUTE.md`.
