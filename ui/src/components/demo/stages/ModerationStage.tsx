import { cn } from "cn"
import { AlertTriangle } from "@untitledui/icons"

import { Progress } from "@/components/ui/progress"
import { Skeleton } from "@/components/ui/skeleton"
import {
  formatScore,
  MODERATION_CATEGORIES,
  MODERATION_FLAG_THRESHOLD,
  type ModerationScores,
} from "@/domain/moderation"
import type { StageState } from "@/domain/stages"

interface ModerationStageProps {
  state: StageState
  scores?: ModerationScores
}

export function ModerationStage({ state, scores }: ModerationStageProps) {
  if (state === "processing") {
    return (
      <div
        className="pipeline-result-enter flex flex-col gap-3"
        role="status"
        aria-label="Scoring"
      >
        {MODERATION_CATEGORIES.map((category, index) => (
          <div
            key={category.key}
            className="pipeline-score-enter flex flex-col gap-1.5"
            style={{ animationDelay: `${index * 55}ms` }}
          >
            <div className="flex items-center justify-between gap-3">
              <Skeleton className="h-4 w-32" />
              <Skeleton className="h-4 w-8" />
            </div>
            <Skeleton className="h-1.5 w-full" />
          </div>
        ))}
        <p className="text-xs text-muted-foreground">
          Checking the transcript across five safety categories.
        </p>
      </div>
    )
  }

  if (state === "failed") {
    return (
      <p className="pipeline-result-enter rounded-lg border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive">
        No category scores were produced.
      </p>
    )
  }

  if (scores === undefined) {
    return <p className="text-sm text-muted-foreground">No scores returned.</p>
  }

  return (
    <div className="flex flex-col gap-3">
      {MODERATION_CATEGORIES.map((category, index) => {
        const value = scores[category.key]
        const isFlagged = value >= MODERATION_FLAG_THRESHOLD
        return (
          <div
            key={category.key}
            className="pipeline-score-enter flex flex-col gap-1.5"
            style={{ animationDelay: `${index * 65}ms` }}
          >
            <div className="flex items-baseline justify-between gap-3 text-sm">
              <span
                className={cn(
                  "inline-flex items-center gap-1.5",
                  isFlagged && "font-medium"
                )}
              >
                {isFlagged ? (
                  <AlertTriangle
                    aria-hidden
                    className="size-3.5 shrink-0 text-warning"
                  />
                ) : null}
                {category.label}
                {isFlagged ? <span className="sr-only"> (flagged)</span> : null}
              </span>
              <span
                className={cn(
                  "font-mono text-xs",
                  isFlagged ? "text-warning" : "text-muted-foreground"
                )}
              >
                {formatScore(value)}
              </span>
            </div>
            <Progress
              value={value * 100}
              aria-label={`${category.label} score`}
              className={cn(
                "h-1.5",
                isFlagged && "[&_[data-slot=progress-indicator]]:bg-warning"
              )}
            />
          </div>
        )
      })}
    </div>
  )
}
