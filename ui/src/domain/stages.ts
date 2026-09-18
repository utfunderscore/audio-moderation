/**
 * The five things the demo visualizes. Each pipeline stage has a partial
 * (started) and a finished state, which is what produces the "processing"
 * card before it flips to "complete".
 */
export const STAGE_IDS = [
  "submitted",
  "conversion",
  "transcription",
  "moderation",
  "result",
] as const

export type StageId = (typeof STAGE_IDS)[number]

export type StageState =
  | "pending"
  | "processing"
  | "complete"
  | "failed"
  | "skipped"

export interface StageDefinition {
  id: StageId
  label: string
  description: string
  /** Events that move this stage into its processing / complete state. */
  startedEvent?: string
  finishedEvent?: string
}

export const STAGES: readonly StageDefinition[] = [
  {
    id: "submitted",
    label: "Submitted",
    description: "Evaluation accepted and dispatched",
    finishedEvent: "EVALUATION_ACCEPTED",
  },
  {
    id: "conversion",
    label: "Conversion",
    description: "Sources downloaded and stitched into one WAV",
    startedEvent: "AUDIO_PROCESSING_STARTED",
    finishedEvent: "AUDIO_PROCESSING_FINISHED",
  },
  {
    id: "transcription",
    label: "Transcription",
    description: "Speech recognised and persisted",
    startedEvent: "ASR_STARTED",
    finishedEvent: "ASR_FINISHED",
  },
  {
    id: "moderation",
    label: "Moderation",
    description: "Five-category scoring over speech and audio",
    startedEvent: "MODERATION_PROCESSING_STARTED",
    finishedEvent: "MODERATION_PROCESSING_FINISHED",
  },
  {
    id: "result",
    label: "Result",
    description: "Terminal workflow outcome",
  },
]

/** Stage timing captured while events arrive, used for per-stage elapsed labels. */
export type StageTimes = Record<
  StageId,
  { startedAt?: number; endedAt?: number }
>

export function createStageTimes(): StageTimes {
  return {
    submitted: {},
    conversion: {},
    transcription: {},
    moderation: {},
    result: {},
  }
}

export function createStages(): Record<StageId, StageState> {
  return {
    submitted: "pending",
    conversion: "pending",
    transcription: "pending",
    moderation: "pending",
    result: "pending",
  }
}
