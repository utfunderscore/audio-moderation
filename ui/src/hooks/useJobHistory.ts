import { useCallback, useEffect, useState } from "react"

import type { AudioProcessingJob } from "@/domain/jobs"
import { inMemoryAudioJobStore } from "@/transport/inMemoryJobStore"

export function useJobHistory(userId: string) {
  const [jobs, setJobs] = useState<AudioProcessingJob[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      setJobs(await inMemoryAudioJobStore.listJobs(userId))
    } catch (fetchError) {
      setError(
        fetchError instanceof Error ? fetchError.message : "Unable to load jobs"
      )
    } finally {
      setLoading(false)
    }
  }, [userId])

  useEffect(() => {
    let cancelled = false

    void inMemoryAudioJobStore
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
  }, [userId])

  const saveJob = useCallback(
    async (job: AudioProcessingJob) => {
      setError(null)
      try {
        await inMemoryAudioJobStore.upsertJob(job)
        await refresh()
      } catch (saveError) {
        setError(
          saveError instanceof Error ? saveError.message : "Unable to save job"
        )
      }
    },
    [refresh]
  )

  return { jobs, loading, error, refresh, saveJob }
}
