import { useCallback, useEffect, useState } from "react"

import type { Backend } from "@/api/backend"
import type { AudioProcessingJob } from "@/domain/jobs"

/** Lists a user's previous jobs through the injected `Backend`. */
export function useJobHistory(backend: Backend, userId: string) {
  const [jobs, setJobs] = useState<AudioProcessingJob[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      setJobs(await backend.listJobs(userId))
    } catch (fetchError) {
      setError(
        fetchError instanceof Error ? fetchError.message : "Unable to load jobs"
      )
    } finally {
      setLoading(false)
    }
  }, [backend, userId])

  useEffect(() => {
    let cancelled = false

    void backend
      .listJobs(userId)
      .then((fetchedJobs) => {
        if (cancelled) return
        setJobs(fetchedJobs)
        setError(null)
      })
      .catch((fetchError: unknown) => {
        if (cancelled) return
        setError(
          fetchError instanceof Error
            ? fetchError.message
            : "Unable to load jobs"
        )
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })

    return () => {
      cancelled = true
    }
  }, [backend, userId])

  return { jobs, loading, error, refresh }
}
