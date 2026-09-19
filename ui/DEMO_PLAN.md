# Audio Moderation Demo Page — UI Plan

Status: draft
Scope: **UI only.** No backend, Terraform, or AWS changes. All pipeline activity is
simulated locally by a mock transport that emits the exact event names the deployed
system emits. The real WebSocket client is stubbed behind an interface so it can be
dropped in later without touching the UI.

## 0. Revision — artifact timeline

The page is a vertical timeline with one page per pipeline stage, and each page
shows the artifact that stage produced:

- **Audio** — choosing a file starts the run immediately. The playable waveform
  and source URI appear only once audio processing completes, when the stitched
  `evaluations/<id>.wav` exists. Before that the page shows a processing state.
- **Transcription** — the transcript text, once `ASR_FINISHED` arrives.
- **Moderation** — the five category scores and the flag verdict, and the end of
  the timeline. A toast reports the terminal outcome; failures render inline on
  the stage that failed.

Transcript and score values are demo fixtures on each scenario, because the real
WebSocket only carries event names — a live integration would read them from the
workflow store. Values already visible on an earlier page are not repeated on a
later one.

The page is customer-facing: raw events, transport status, debug scenarios, and
other test-harness controls are intentionally not shown.

## 1. Goal

A single-page demonstration that takes an audio file and visualizes the moderation
pipeline advancing stage by stage, driven by the same event sequence the production
WebSocket delivers.

The page must demonstrate three things:

1. **Stages** — the pipeline moves through audio conversion, transcription, and
   moderation before producing a result.
2. **Partial stage changes** — a stage is visibly "processing" before it becomes
   "complete" (`*_STARTED` then `*_FINISHED`).
3. **Synchronized state** — the stage tracker is entirely derived from a stream of
   event names, including replay after a late subscribe and duplicate frames.

## 2. Pipeline model extracted from the backend

Happy path, exactly the order the system emits these (`SUCCESSFUL_LIFECYCLE_EVENTS`
in `backend/crates/start-evaluation-lambda/tests/deployed.rs`):

| Stage | Start event | Finish event | Emitted by |
|---|---|---|---|
| Submitted | — (`EVALUATION_ACCEPTED`) | — | `start-evaluation-lambda/src/service.rs` |
| Conversion | `AUDIO_PROCESSING_STARTED` | `AUDIO_PROCESSING_FINISHED` | `audio-processing-lambda/src/lib.rs` |
| Transcription | `ASR_STARTED` | `ASR_FINISHED` | `transcription-caller-lambda/src/lib.rs`, `task-callback-lambda/src/lib.rs` |
| Moderation | `MODERATION_PROCESSING_STARTED` | `MODERATION_PROCESSING_FINISHED` | `moderation-caller-lambda/src/lib.rs`, `task-callback-lambda/src/lib.rs` |
| Result | `SUCCEEDED` | — | `audio-processing-lambda/src/main.rs` |

Terminal error events can arrive instead of `SUCCEEDED`, at almost any point:
`FAILED` (ingress, conversion, either caller, callback, lifecycle), `TIMED_OUT`,
`CANCELLED`.

Ingress also returns an `evaluationId` and a pipeline status
(`PIPELINE_TASK_STATUS_*`, from `proto/audio/moderation/v1/audio_moderation.proto`)
that the UI seeds from before the socket replay arrives.

The deployed transport contract (from `task-events-lambda/src/lib.rs` and
`task-event-emitter/src/lib.rs`) that a future real adapter must honor:

- One socket maps to one task. The deployed client authorizes its evaluation
  access token with `CreateTaskEventsTicket`, then subscribes with
  `{"action":"subscribe","ticket":<ticket>}`. The current mock interface and
  unwired real adapter still use `taskId`; that is simulation-only and does not
  match the deployed protocol.
- Frames are raw UTF-8 event names. No payloads, IDs, timestamps, or error text.
- On subscribe the server replays durable history in event order, then live events.
  Replay and live frames can interleave, and delivery is at-least-once → the client
  must dedupe by event name and must never regress a completed stage.
- The durable event set is unique per task (`UNIQUE (task_id, event_name)`), so a
  name-based dedupe is sufficient.

## 3. Screen design

Single route, three panels plus a control bar.

```
┌──────────────────────────────────────────────────────────────────┐
│ SocialGuard · Audio Moderation Demo      [Simulation] ● connected│
├───────────────────┬──────────────────────────────────────────────┤
│ AUDIO INPUT       │ PIPELINE                                     │
│ drop zone / picker│  Submitted → Conversion → ASR → Moderation → │
│ waveform + player │  Result                                      │
│ duration / format │  (stage cards, connectors animate)           │
│ "Use sample"      │                                              │
│ [Run evaluation]  │  ── terminal banner on success/failure ──    │
├───────────────────┴──────────────────────────────────────────────┤
│ EVENT LOG (raw frames)                    [scenario ▾] [speed]   │
│ 12:00:01.204  EVALUATION_ACCEPTED                                │
│ 12:00:01.210  AUDIO_PROCESSING_STARTED                           │
│ ...                                                              │
└──────────────────────────────────────────────────────────────────┘
```

### Audio input panel

- Drag-and-drop or file picker (`accept="audio/*"`). Local file is never uploaded;
  this is stated in the panel copy.
- `<audio>` element with native controls.
- Client-side metadata shown: filename, size, duration, sample rate, channel count.
  Decode with `AudioContext.decodeAudioData` to draw a lightweight waveform
  (down-sampled peaks to `<canvas>`); render a static bar strip if decoding fails.
- "Use sample" loads `ui/public/audio/sample_071.mp3` (copied from the repo fixture).
- While running, the panel shows a synthetic source URI (`s3://demo/uploads/<name>`)
  so the demo reads like the real API contract without implying a real upload.

### Stage tracker

Five stage cards: Submitted, Conversion, Transcription, Moderation, Result.

Each card has one of:

- `pending` — dimmed.
- `processing` — amber, pulsing indicator, running elapsed timer. This is the
  "partial" state produced by `*_STARTED`.
- `complete` — green check, final elapsed/duration.
- `failed` — red, shown on the stage that was processing when a terminal error event
  arrived.
- `skipped` — dimmed with a dash, for stages never reached after a failure.

Connectors between cards animate when the downstream stage activates. The Result
card doubles as the terminal banner: succeeded / failed / timed out / cancelled.

### Event log

- One row per received frame: receive timestamp, raw event name, and a `replay` or
  `duplicate` badge where applicable.
- Newest at the bottom; auto-scroll with a "pause scroll" toggle.
- Terminal events get a distinct row style.
- This panel is the evidence that the dashboard is event-driven; it stays visible in
  every scenario.

### Controls

- Scenario selector (see §6).
- Speed: 1x / 4x / instant.
- Run, Restart, and a "Subscribe late" toggle (delivers the first N events as replay
  after subscribing instead of live).
- Connection indicator: `idle → connecting → subscribed → closed`.

## 4. State model

Framework-agnostic domain layer; React only renders it.

```ts
type StageId = 'submitted' | 'conversion' | 'transcription' | 'moderation' | 'result';

type StageState = 'pending' | 'processing' | 'complete' | 'failed' | 'skipped';

type TerminalOutcome = 'SUCCEEDED' | 'FAILED' | 'TIMED_OUT' | 'CANCELLED';

interface ReceivedEvent {
  name: string;
  receivedAt: number;
  source: 'replay' | 'live';
  duplicate: boolean;
}

interface PipelineState {
  evaluationId: string | null;
  stages: Record<StageId, StageState>;
  outcome: TerminalOutcome | null;
  events: ReceivedEvent[];
  connection: 'idle' | 'connecting' | 'subscribed' | 'closed';
}
```

`applyEvent(state, name, meta)` rules, in order:

1. **Unknown name** → append to log, no state change. Forward compatible with new
   backend events.
2. **Duplicate name** → append flagged `duplicate`, no state change.
3. **Known name** → append, then apply the mapping below.
4. **Monotonicity** — a stage may only move `pending → processing → complete`, or to
   `failed`. It never regresses, so replay/live interleaving is safe.
5. **Terminal name** → set `outcome`, mark the currently `processing` stage `failed`
   (or `submitted` failed for ingress failures), and every still-`pending` stage
   `skipped`.

Event → transition table:

| Event | Transition |
|---|---|
| `EVALUATION_ACCEPTED` | submitted → complete |
| `AUDIO_PROCESSING_STARTED` | conversion → processing |
| `AUDIO_PROCESSING_FINISHED` | conversion → complete |
| `ASR_STARTED` | transcription → processing |
| `ASR_FINISHED` | transcription → complete |
| `MODERATION_PROCESSING_STARTED` | moderation → processing |
| `MODERATION_PROCESSING_FINISHED` | moderation → complete |
| `SUCCEEDED` | result → complete, outcome = SUCCEEDED |
| `FAILED` / `TIMED_OUT` / `CANCELLED` | active stage → failed, pending stages → skipped, outcome set |

Seeding: after the (mock) ingress call the reducer is initialized from the returned
`evaluationId` + status, e.g. `PIPELINE_TASK_STATUS_STARTED_ASR` seeds submitted and
conversion complete, transcription processing. In the mock, the scenario chooses the
seed to demonstrate late subscription. Replay frames then reconcile.

## 5. Transport seam (mock now, real later)

```ts
interface TaskEventsClient {
  connect(): Promise<void>;
  subscribe(taskId: number): Promise<void>;
  close(): void;
  onFrame(handler: (event: string, source: 'replay' | 'live') => void): () => void;
  onConnectionChange(handler: (state: ConnectionState) => void): () => void;
}
```

- `MockTaskEventsClient` — the only implementation wired into the app. It plays a
  scenario script: each step is `{ event, delayMs, source? }`. On `subscribe` it
  delivers the script's replay prefix, then schedules the live remainder. It
  supports injected duplicates, connection drops, and re-subscription.
- `WebSocketTaskEventsClient` — an unwired, stale stub. The deployed handshake is
  `wss://…`, authorized `CreateTaskEventsTicket`, then
  `{"action":"subscribe","ticket":...}` with text-or-binary UTF-8 frames,
  replay-then-live, at-least-once delivery, and `$disconnect` cleanup. The stub
  still sends `taskId`, cannot mint a ticket, and is not tested in this phase.
- The app talks only to the interface through `usePipelineRun`, so switching
  implementations is a one-line change plus an endpoint/`evaluationId` source.

## 6. Mock scenarios

Each scenario is a fixture in `src/transport/scenarios.ts`, derived from real event
sequences:

| Scenario | Sequence | Demonstrates |
|---|---|---|
| Happy path | all 8 events, realistic delays (conversion ~2s, ASR ~6s, moderation ~4s) | full stage progression |
| Late subscribe | first 4 events delivered as replay on subscribe, rest live | replay reconciliation |
| Duplicate boundary | one event repeated across the replay/live boundary | idempotent dedupe |
| Slow ASR | `ASR_STARTED` held for ~20s | partial stage state, elapsed timer |
| Conversion failure | `EVALUATION_ACCEPTED`, `AUDIO_PROCESSING_STARTED`, `FAILED` | stage-level failure |
| ASR failure | `… ASR_STARTED`, `FAILED` | mid-pipeline failure |
| Moderation failure | `… MODERATION_PROCESSING_STARTED`, `FAILED` | late-stage failure |
| Timeout | `… MODERATION_PROCESSING_STARTED`, `TIMED_OUT` | timeout distinct from failure |
| Cancelled | `… ASR_STARTED`, `CANCELLED` | cancellation |

Unknown-event resilience is covered by a `future-event` scenario that injects a name
outside the known set.

## 7. File layout

```
ui/
  DEMO_PLAN.md
  DEMO_DESIGN_ROUTE.md
  components.json
  index.html
  package.json
  tsconfig.json
  tsconfig.app.json
  vite.config.ts
  public/audio/sample_071.mp3
  src/
    main.tsx
    App.tsx
    index.css            # Tailwind v4 + shadcn/status tokens
    domain/
      events.ts            # known event names, terminal set
      stages.ts            # stage definitions, order, labels, mapping
      reducer.ts           # init, applyEvent, selectors
      reducer.test.ts
    transport/
      client.ts            # TaskEventsClient interface + types
      mockClient.ts        # scripted mock implementation
      scenarios.ts         # scenario fixtures
      webSocketClient.ts   # real adapter stub, unwired
    hooks/
      usePipelineRun.ts    # ingress → subscribe → dispatch loop
      useElapsed.ts
    components/
      ui/                  # generated shadcn components (Button, Card, …)
      demo/
        Header.tsx
        AudioInputPanel.tsx
        Waveform.tsx
        StageTracker.tsx
        StageCard.tsx
        RunControls.tsx
        TerminalBanner.tsx
```

Stack: Vite + React + TypeScript, Tailwind CSS v4, and shadcn/ui with Untitled UI
icons. See `DEMO_DESIGN_ROUTE.md` for the component/icon mapping, layout route, and
status tokens. Vitest for the reducer tests only — the domain layer is where
correctness lives.

## 8. Visual polish checklist

- Indeterminate progress on the active stage; no fake percentage (the backend exposes
  none).
- Overall elapsed timer once a run starts.
- Elapsed-per-stage timers so long ASR/moderation waits feel intentional.
- Stage transitions animate (fade/slide, connector fill).
- `aria-live="polite"` announcements for stage transitions; event log is a semantic
  list; controls are keyboard reachable; honor `prefers-reduced-motion`.
- Empty state before the first run explains the demo in two sentences.
- Simulation badge always visible so nobody mistakes the page for a live integration.

## 9. Build phases

1. **Scaffold** — Vite + React + TS in `ui/`, base layout, theme tokens, sample audio
   copied to `public/audio/`.
2. **Domain** — `events.ts`, `stages.ts`, `reducer.ts` + tests for happy path,
   duplicates, out-of-order replay/live interleave, unknown events, every terminal
   event.
3. **Transport** — client interface, mock client, all scenarios.
4. **Components** — audio panel + waveform, stage tracker, event log, controls,
   terminal banner; wire `usePipelineRun`.
5. **Polish** — animations, timers, a11y pass, responsive layout, README with run
   instructions.

## 10. Out of scope (recorded for later)

- Real `StartEvaluation` / `SubmitReview` calls, S3 upload, CORS, and Terraform.
- Real `WebSocketTaskEventsClient` wiring and reconnect/backoff.
- Reading transcript or moderation scores — the public API has no read RPC today;
  the UI will not invent one. If the demo later needs results, that is a backend
  change (`GetEvaluation`) rather than a UI change.
- Auth, multi-task views, persistence, anything deploy-related.
