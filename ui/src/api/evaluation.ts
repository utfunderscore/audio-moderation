import type { ModerationScores } from "@/domain/moderation"

export interface StartEvaluationInput {
  userId: string
  audio: File
  /** Single-use Turnstile response for this submission attempt (never persisted). */
  turnstileToken: string
}

export interface StartedEvaluation {
  evaluationId: string
  /** `PIPELINE_TASK_STATUS_*` returned by the API. */
  status: string
}

export interface ResumeEvaluationInput {
  evaluationId: string
  startedAt: number
}

export interface EvaluationResult {
  transcript?: string
  scores?: ModerationScores
}
