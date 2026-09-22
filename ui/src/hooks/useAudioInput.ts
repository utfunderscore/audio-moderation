import { useCallback, useEffect, useRef, useState } from "react"

export type AudioInputController = ReturnType<typeof useAudioInput>

export interface InitialAudioInput {
  file: File
  url: string
}

/**
 * Owns the local audio file and its object URL. Playback subscriptions live
 * with the player so media-time updates do not invalidate the application
 * tree. Uploading the file to the backend is the caller's responsibility.
 */
export function useAudioInput(initialAudio?: InitialAudioInput) {
  const audioRef = useRef<HTMLAudioElement | null>(null)
  const [file, setFile] = useState<File | null>(initialAudio?.file ?? null)
  const [url, setUrl] = useState<string | null>(initialAudio?.url ?? null)

  useEffect(() => {
    if (url === null) return undefined
    return () => {
      URL.revokeObjectURL(url)
    }
  }, [url])

  const selectFile = useCallback((next: File) => {
    audioRef.current?.pause()
    setFile(next)
    setUrl(URL.createObjectURL(next))
  }, [])

  const clear = useCallback(() => {
    audioRef.current?.pause()
    setFile(null)
    setUrl(null)
  }, [])

  return {
    audioRef,
    file,
    url,
    selectFile,
    clear,
  }
}
