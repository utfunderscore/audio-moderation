import type { AudioProcessingJob } from "@/domain/jobs"
import type { ModerationScores } from "@/domain/moderation"
import type { ConnectionState } from "@/domain/reducer"

/**
 * The single seam between the UI and the backend.
 *
 * Every backend operation the product UI performs is declared here. Components
 * and hooks depend only on this interface, so the transport — HTTP, WebSocket,
 * uploads, auth, retries — lives entirely in the implementation. Provide one
 * at the composition root (`src/main.tsx`) to connect the UI.
 *
 * `subscribeTaskEvents` carries event names only, so `getEvaluationResult`
 * supplies the transcript and moderation scores once a run finishes. Because
 * the server has no list endpoint, terminal snapshots are also recorded in the
 * browser-local history through this seam.
 */
export interface Backend {
  /** Jobs previously submitted by this user, newest first. */
  listJobs(userId: string): Promise<AudioProcessingJob[]>

  /** Upload the selected audio and start the evaluation pipeline. */
  startEvaluation(input: StartEvaluationInput): Promise<StartedEvaluation>

  /** Persisted transcript and scores for a finished evaluation, if any. */
  getEvaluationResult(evaluationId: string): Promise<EvaluationResult | null>

  /** Audio for a job opened from the history list. */
  getJobAudio(jobId: string): Promise<Blob>

  /**
   * Open the evaluation's task-events stream and return an unsubscribe
   * function. Connection failures surface through `handlers.onError`.
   */
  subscribeTaskEvents(
    evaluationId: string,
    handlers: TaskEventHandlers
  ): Unsubscribe
}

export interface StartEvaluationInput {
  userId: string
  audio: File
}

export interface StartedEvaluation {
  evaluationId: string
  /** `PIPELINE_TASK_STATUS_*` returned at ingress; seeds the stage tracker. */
  status: string
}

export interface EvaluationResult {
  transcript?: string
  scores?: ModerationScores
}

/** One task-events frame. The stream carries event names only. */
export interface TaskEventFrame {
  name: string
}

export interface TaskEventHandlers {
  onFrame: (frame: TaskEventFrame) => void
  onConnectionChange?: (state: ConnectionState) => void
  onError?: (error: unknown) => void
}

export type Unsubscribe = () => void
