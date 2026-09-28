import { useEffect, useState } from "react"

/** Ticks once a second while `active` so elapsed labels stay live. */
export function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now())

  useEffect(() => {
    if (!active) return undefined
    setNow(Date.now())
    const id = window.setInterval(() => setNow(Date.now()), 1_000)
    return () => window.clearInterval(id)
  }, [active])

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
