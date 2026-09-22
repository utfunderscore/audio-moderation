import { isFlagged, type ModerationScores } from "@/domain/moderation"

export type AudioJobStatus = "processing" | "complete" | "failed"

/**
 * Status shown in the jobs list. A completed job whose moderation scores cross
 * the flag threshold is surfaced as "flagged" instead of "complete".
 */
export type AudioJobDisplayStatus = AudioJobStatus | "flagged"

/** Resolves the list status for a job, folding in its moderation outcome. */
export function jobDisplayStatus(
  status: AudioJobStatus,
  scores?: ModerationScores
): AudioJobDisplayStatus {
  return status === "complete" && isFlagged(scores) ? "flagged" : status
}

export interface AudioProcessingJob {
  id: string
  userId: string
  fileName: string
  submittedAt: number
  durationMs: number | null
  status: AudioJobStatus
  transcript?: string
  scores?: ModerationScores
}

/** Placeholder identity until the UI is connected to authentication. */
export const DEMO_USER_ID = "demo-user"
