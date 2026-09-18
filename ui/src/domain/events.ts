/**
 * Event names emitted by the production pipeline and delivered as raw UTF-8
 * frames over the task-events WebSocket.
 *
 * Keep this list in sync with the emitters in the backend:
 * - start-evaluation-lambda: EVALUATION_ACCEPTED, FAILED (dispatch exhausted)
 * - audio-processing-lambda: AUDIO_PROCESSING_STARTED/FINISHED, SUCCEEDED, FAILED
 * - transcription-caller / task-callback: ASR_STARTED, ASR_FINISHED
 * - moderation-caller / task-callback: MODERATION_PROCESSING_STARTED/FINISHED
 * - workflow lifecycle: TIMED_OUT, CANCELLED
 */

/** The ordered happy-path sequence, exactly as the deployed system emits it. */
export const LIFECYCLE_EVENTS = [
  "EVALUATION_ACCEPTED",
  "AUDIO_PROCESSING_STARTED",
  "AUDIO_PROCESSING_FINISHED",
  "ASR_STARTED",
  "ASR_FINISHED",
  "MODERATION_PROCESSING_STARTED",
  "MODERATION_PROCESSING_FINISHED",
  "SUCCEEDED",
] as const

export type LifecycleEvent = (typeof LIFECYCLE_EVENTS)[number]

export const TERMINAL_EVENTS = [
  "SUCCEEDED",
  "FAILED",
  "TIMED_OUT",
  "CANCELLED",
] as const

export type TerminalEvent = (typeof TERMINAL_EVENTS)[number]

export const FAILURE_EVENTS = ["FAILED", "TIMED_OUT", "CANCELLED"] as const

export type FailureEvent = (typeof FAILURE_EVENTS)[number]

export type KnownEvent = LifecycleEvent | TerminalEvent

export function isTerminalEvent(name: string): name is TerminalEvent {
  return (TERMINAL_EVENTS as readonly string[]).includes(name)
}

export function isKnownEvent(name: string): name is KnownEvent {
  return (
    (LIFECYCLE_EVENTS as readonly string[]).includes(name) ||
    (TERMINAL_EVENTS as readonly string[]).includes(name)
  )
}
