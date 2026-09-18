import { useEffect, useState } from "react"

/** Ticks while `active` so elapsed labels and the run timer stay live. */
export function useNow(active: boolean, intervalMs = 250): number {
  const [now, setNow] = useState(() => Date.now())

  useEffect(() => {
    if (!active) return undefined
    const id = window.setInterval(() => setNow(Date.now()), intervalMs)
    return () => window.clearInterval(id)
  }, [active, intervalMs])

  return now
}

export function useElapsed(
  startedAt: number | null,
  active: boolean
): number | null {
  const now = useNow(active)
  if (startedAt === null) return null
  return now - startedAt
}
