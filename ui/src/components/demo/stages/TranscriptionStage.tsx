import { Skeleton } from "@/components/ui/skeleton"
import type { StageState } from "@/domain/stages"

interface TranscriptionStageProps {
  state: StageState
  transcript?: string
}

function countWords(text: string): number {
  return text.trim().split(/\s+/).filter(Boolean).length
}

export function TranscriptionStage({
  state,
  transcript,
}: TranscriptionStageProps) {
  if (state === "processing") {
    return (
      <div
        className="pipeline-result-enter flex flex-col gap-3"
        role="status"
        aria-label="Transcribing"
      >
        <div className="flex flex-col gap-2 rounded-lg border bg-card p-4">
          <Skeleton className="h-4 w-full" />
          <Skeleton className="h-4 w-11/12" />
          <Skeleton className="h-4 w-3/4" />
        </div>
        <p className="text-xs text-muted-foreground">
          Converting speech to text. The transcript will appear when it is
          ready.
        </p>
      </div>
    )
  }

  if (state === "failed") {
    return (
      <p className="pipeline-result-enter rounded-lg border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive">
        No transcript was produced.
      </p>
    )
  }

  if (transcript === undefined) {
    return (
      <p className="text-sm text-muted-foreground">No transcript returned.</p>
    )
  }

  return (
    <div className="pipeline-result-enter flex flex-col gap-3">
      <blockquote className="rounded-lg border bg-card p-4 text-sm leading-relaxed">
        {transcript}
      </blockquote>
      <p className="text-xs text-muted-foreground">
        {countWords(transcript)} words
      </p>
    </div>
  )
}
