import { formatDuration } from "@/domain/reducer"
import { useNow } from "@/hooks/useElapsed"

interface ElapsedDurationProps {
  startedAt: number | null | undefined
  endedAt?: number
  placeholder?: string
}

/** A live duration label whose timer is contained to this text node. */
export function ElapsedDuration({
  startedAt,
  endedAt,
  placeholder = "—",
}: ElapsedDurationProps) {
  const now = useNow(
    startedAt !== null && startedAt !== undefined && endedAt === undefined
  )

  if (startedAt === null || startedAt === undefined) return placeholder
  return formatDuration((endedAt ?? now) - startedAt)
}
