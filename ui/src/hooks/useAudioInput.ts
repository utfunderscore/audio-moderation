import { useCallback, useEffect, useRef, useState } from "react"

export type AudioInputController = ReturnType<typeof useAudioInput>

export interface InitialAudioInput {
  file: File
  url: string
}

/**
 * Owns the local audio file and source URL. Playback subscriptions live with
 * the player so media time updates do not invalidate the application tree.
 * The file never leaves the browser in simulation mode.
 */
export function useAudioInput(initialAudio?: InitialAudioInput) {
  const audioRef = useRef<HTMLAudioElement | null>(null)
  const [file, setFile] = useState<File | null>(initialAudio?.file ?? null)
  const [url, setUrl] = useState<string | null>(initialAudio?.url ?? null)
  const [loadingSample, setLoadingSample] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (url === null) return undefined
    return () => {
      URL.revokeObjectURL(url)
    }
  }, [url])

  const selectFile = useCallback((next: File) => {
    audioRef.current?.pause()
    setFile(next)
    setError(null)
    setUrl(URL.createObjectURL(next))
  }, [])

  const clear = useCallback(() => {
    audioRef.current?.pause()
    setFile(null)
    setUrl(null)
    setError(null)
  }, [])

  const useSample = useCallback(async () => {
    setLoadingSample(true)
    setError(null)
    try {
      const response = await fetch(
        `${import.meta.env.BASE_URL}audio/sample_071.mp3`
      )
      if (!response.ok) {
        throw new Error(`sample audio unavailable (${response.status})`)
      }
      const blob = await response.blob()
      selectFile(
        new File([blob], "sample_071.mp3", {
          type: blob.type || "audio/mpeg",
        })
      )
      return true
    } catch (sampleError) {
      setError(
        sampleError instanceof Error
          ? sampleError.message
          : "failed to load the sample"
      )
      return false
    } finally {
      setLoadingSample(false)
    }
  }, [selectFile])

  return {
    audioRef,
    file,
    url,
    loadingSample,
    error,
    selectFile,
    useSample,
    clear,
    canRun: file !== null,
  }
}
