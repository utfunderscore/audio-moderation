/**
 * Moderation output contract.
 *
 * The real WebSocket never carries these values — it only delivers event names.
 * The UI reads them from the workflow's persisted results
 * (`moderation_tasks` / `transcription_tasks`) through
 * `Backend.getEvaluationResult`.
 */
export interface ModerationScores {
  sexual: number
  hate_or_discrimination: number
  harassment_or_abuse: number
  violence_or_threats: number
  asking_for_pii: number
}

export interface ModerationCategory {
  key: keyof ModerationScores
  label: string
}

export const MODERATION_CATEGORIES: readonly ModerationCategory[] = [
  { key: "sexual", label: "Sexual" },
  { key: "hate_or_discrimination", label: "Hate or discrimination" },
  { key: "harassment_or_abuse", label: "Harassment or abuse" },
  { key: "violence_or_threats", label: "Violence or threats" },
  { key: "asking_for_pii", label: "Asking for PII" },
]

/** Inclusive score at or above which a category is called out in the demo. */
export const MODERATION_FLAG_THRESHOLD = 0.5

export function flaggedCategories(
  scores: ModerationScores
): ModerationCategory[] {
  return MODERATION_CATEGORIES.filter(
    (category) => scores[category.key] >= MODERATION_FLAG_THRESHOLD
  )
}

/** True when at least one category is at or above the flag threshold. */
export function isFlagged(scores?: ModerationScores): boolean {
  return scores !== undefined && flaggedCategories(scores).length > 0
}

export function formatScore(value: number): string {
  return `${Math.round(value * 100)}%`
}
