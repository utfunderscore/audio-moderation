import { useCallback, useSyncExternalStore } from "react"

/**
 * Subscribes to a CSS media query. `useSyncExternalStore` owns the
 * subscribe/unsubscribe lifecycle, so callers do not need an Effect.
 */
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (onStoreChange: () => void) => {
      const mediaQuery = window.matchMedia(query)
      mediaQuery.addEventListener("change", onStoreChange)
      return () => mediaQuery.removeEventListener("change", onStoreChange)
    },
    [query]
  )

  const getSnapshot = useCallback(
    () => window.matchMedia(query).matches,
    [query]
  )

  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot)
}
