import { isTerminalEvent, type TerminalEvent } from "./events"
import {
  createStages,
  createStageTimes,
  STAGE_IDS,
  type StageId,
  type StageState,
  type StageTimes,
} from "./stages"

/** Connection lifecycle as reported by the (mock or real) task-events client. */
export type ConnectionState =
  | "idle"
  | "connecting"
  | "subscribed"
  | "closed"
  | "error"

export type EventSource = "replay" | "live"

export interface ReceivedEvent {
  name: string
  receivedAt: number
  source: EventSource
  duplicate: boolean
}

export interface PipelineState {
  evaluationId: string | null
  /** Initial PIPELINE_TASK_STATUS_* for the simulated pipeline task. */
  ingressStatus: string | null
  stages: Record<StageId, StageState>
  stageTimes: StageTimes
  outcome: TerminalEvent | null
  events: ReceivedEvent[]
  connection: ConnectionState
  startedAt: number | null
}

export type PipelineAction =
  | { type: "reset" }
  | { type: "seed"; evaluationId: string; status: string; at: number }
  | { type: "connection"; state: ConnectionState }
  | { type: "frame"; name: string; source: EventSource; at: number }
  | { type: "clear-events" }

/**
 * Pipeline status values from proto/audio/moderation/v1/audio_moderation.proto,
 * used to seed the tracker before replay lands.
 */
export const PIPELINE_STATUS = {
  pending: "PIPELINE_TASK_STATUS_PENDING",
  startedAudioProcessing: "PIPELINE_TASK_STATUS_STARTED_AUDIO_PROCESSING",
  audioProcessingFinished: "PIPELINE_TASK_STATUS_AUDIO_PROCESSING_FINISHED",
  startedAsr: "PIPELINE_TASK_STATUS_STARTED_ASR",
  asrFinished: "PIPELINE_TASK_STATUS_ASR_FINISHED",
  startedModerationProcessing:
    "PIPELINE_TASK_STATUS_STARTED_MODERATION_PROCESSING",
  moderationProcessingFinished:
    "PIPELINE_TASK_STATUS_MODERATION_PROCESSING_FINISHED",
  succeeded: "PIPELINE_TASK_STATUS_SUCCEEDED",
  failed: "PIPELINE_TASK_STATUS_FAILED",
  timedOut: "PIPELINE_TASK_STATUS_TIMED_OUT",
  cancelled: "PIPELINE_TASK_STATUS_CANCELLED",
} as const

export function createInitialState(): PipelineState {
  return {
    evaluationId: null,
    ingressStatus: null,
    stages: createStages(),
    stageTimes: createStageTimes(),
    outcome: null,
    events: [],
    connection: "idle",
    startedAt: null,
  }
}

const STAGE_PROGRESS: StageState[] = [
  "pending",
  "processing",
  "complete",
  "failed",
]

/**
 * Monotonic stage transition. A stage only ever advances, so replay and live
 * frames arriving out of order can never regress the tracker. Terminal failure
 * is the one override that may replace an already-complete stage.
 */
function advanceState(current: StageState, next: StageState): StageState {
  if (current === "failed" || current === "skipped") return current
  if (next === "skipped") return current === "pending" ? "skipped" : current
  if (next === "failed") return "failed"
  return STAGE_PROGRESS.indexOf(next) > STAGE_PROGRESS.indexOf(current)
    ? next
    : current
}

function applyStage(
  state: PipelineState,
  id: StageId,
  next: StageState,
  at: number
): PipelineState {
  const advanced = advanceState(state.stages[id], next)
  if (advanced === state.stages[id]) return state

  const stages = { ...state.stages, [id]: advanced }
  const times = { ...state.stageTimes, [id]: { ...state.stageTimes[id] } }

  if (advanced === "processing" && times[id].startedAt === undefined) {
    times[id].startedAt = at
  }
  if (advanced === "complete" || advanced === "failed") {
    if (times[id].startedAt === undefined) times[id].startedAt = at
    if (times[id].endedAt === undefined) times[id].endedAt = at
  }

  return { ...state, stages, stageTimes: times }
}

function firstProcessingStage(
  stages: Record<StageId, StageState>
): StageId | null {
  for (const id of STAGE_IDS) {
    if (stages[id] === "processing") return id
  }
  return null
}

function applyTerminal(
  state: PipelineState,
  outcome: TerminalEvent,
  at: number
): PipelineState {
  let next = applyStage(
    state,
    "result",
    outcome === "SUCCEEDED" ? "complete" : "failed",
    at
  )

  if (outcome !== "SUCCEEDED") {
    const active = firstProcessingStage(next.stages)
    if (active !== null) {
      next = applyStage(next, active, "failed", at)
    } else if (next.stages.submitted === "complete") {
      next = applyStage(next, "submitted", "failed", at)
    }

    const stages = { ...next.stages }
    for (const id of STAGE_IDS) {
      if (id !== "result" && stages[id] === "pending") stages[id] = "skipped"
    }
    next = { ...next, stages }
  }

  return { ...next, outcome }
}

function applyKnownEvent(
  state: PipelineState,
  name: string,
  at: number
): PipelineState {
  switch (name) {
    case "EVALUATION_ACCEPTED":
      return applyStage(state, "submitted", "complete", at)
    case "AUDIO_PROCESSING_STARTED":
      return applyStage(state, "conversion", "processing", at)
    case "AUDIO_PROCESSING_FINISHED":
      return applyStage(state, "conversion", "complete", at)
    case "ASR_STARTED":
      return applyStage(state, "transcription", "processing", at)
    case "ASR_FINISHED":
      return applyStage(state, "transcription", "complete", at)
    case "MODERATION_PROCESSING_STARTED":
      return applyStage(state, "moderation", "processing", at)
    case "MODERATION_PROCESSING_FINISHED":
      return applyStage(state, "moderation", "complete", at)
    default:
      return state
  }
}

function seedStageStates(status: string): Partial<Record<StageId, StageState>> {
  switch (status) {
    case PIPELINE_STATUS.startedAudioProcessing:
      return { submitted: "complete", conversion: "processing" }
    case PIPELINE_STATUS.audioProcessingFinished:
      return { submitted: "complete", conversion: "complete" }
    case PIPELINE_STATUS.startedAsr:
      return {
        submitted: "complete",
        conversion: "complete",
        transcription: "processing",
      }
    case PIPELINE_STATUS.asrFinished:
      return {
        submitted: "complete",
        conversion: "complete",
        transcription: "complete",
      }
    case PIPELINE_STATUS.startedModerationProcessing:
      return {
        submitted: "complete",
        conversion: "complete",
        transcription: "complete",
        moderation: "processing",
      }
    case PIPELINE_STATUS.moderationProcessingFinished:
      return {
        submitted: "complete",
        conversion: "complete",
        transcription: "complete",
        moderation: "complete",
      }
    case PIPELINE_STATUS.succeeded:
      return {
        submitted: "complete",
        conversion: "complete",
        transcription: "complete",
        moderation: "complete",
        result: "complete",
      }
    default:
      return {}
  }
}

function seedOutcome(status: string): TerminalEvent | null {
  switch (status) {
    case PIPELINE_STATUS.succeeded:
      return "SUCCEEDED"
    case PIPELINE_STATUS.failed:
      return "FAILED"
    case PIPELINE_STATUS.timedOut:
      return "TIMED_OUT"
    case PIPELINE_STATUS.cancelled:
      return "CANCELLED"
    default:
      return null
  }
}

function seed(
  state: PipelineState,
  evaluationId: string,
  status: string,
  at: number
): PipelineState {
  const stages = { ...createStages(), ...seedStageStates(status) }
  const times = createStageTimes()

  for (const id of STAGE_IDS) {
    if (stages[id] === "pending") continue
    const ended = stages[id] === "complete" || stages[id] === "failed"
    times[id] = { startedAt: at, endedAt: ended ? at : undefined }
  }

  return {
    ...state,
    evaluationId,
    ingressStatus: status,
    stages,
    stageTimes: times,
    outcome: seedOutcome(status),
    startedAt: at,
  }
}

export function pipelineReducer(
  state: PipelineState,
  action: PipelineAction
): PipelineState {
  switch (action.type) {
    case "reset":
      return createInitialState()

    case "clear-events":
      return { ...state, events: [] }

    case "connection":
      return { ...state, connection: action.state }

    case "seed":
      return seed(state, action.evaluationId, action.status, action.at)

    case "frame": {
      const { name, source, at } = action
      const duplicate = state.events.some((event) => event.name === name)
      const next: PipelineState = {
        ...state,
        events: [...state.events, { name, receivedAt: at, source, duplicate }],
      }

      // A duplicate frame carries no new information, and a terminal state is
      // final: keep both out of the stage machine.
      if (duplicate || next.outcome !== null) return next

      if (isTerminalEvent(name)) return applyTerminal(next, name, at)
      return applyKnownEvent(next, name, at)
    }

    default:
      return state
  }
}

export function activeStageId(state: PipelineState): StageId | null {
  return firstProcessingStage(state.stages)
}

export function isRunning(state: PipelineState): boolean {
  return state.startedAt !== null && state.outcome === null
}

export function isTerminal(state: PipelineState): boolean {
  return state.outcome !== null
}

/** Elapsed milliseconds for a stage, frozen once the stage ends. */
export function stageElapsed(
  state: PipelineState,
  id: StageId,
  now: number
): number | null {
  const times = state.stageTimes[id]
  if (times.startedAt === undefined) return null
  return (times.endedAt ?? now) - times.startedAt
}

export function runElapsed(state: PipelineState, now: number): number | null {
  if (state.startedAt === null) return null
  return now - state.startedAt
}

export function formatDuration(ms: number | null): string {
  if (ms === null) return "—"
  const totalSeconds = Math.max(0, Math.round(ms / 100) / 10)
  if (totalSeconds < 60) return `${totalSeconds.toFixed(1)}s`
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = Math.round(totalSeconds % 60)
  return `${minutes}m ${seconds}s`
}
