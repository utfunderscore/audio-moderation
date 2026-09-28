import { Skeleton } from "@/components/ui/skeleton"

/**
 * Shown between upload and the end of audio processing: the stitched artifact
 * does not exist yet, so the player is not available.
 */
export function AudioProcessing({ fileName }: { fileName: string }) {
  return (
    <div
      className="flex flex-col gap-4"
      role="status"
      aria-label="Processing audio"
    >
      <div className="flex items-center gap-3">
        <div className="min-w-0">
          <p className="truncate text-sm font-medium">{fileName}</p>
          <p className="text-xs text-muted-foreground">Preparing your audio…</p>
        </div>
      </div>
      <Skeleton className="h-16 w-full" />
    </div>
  )
}
