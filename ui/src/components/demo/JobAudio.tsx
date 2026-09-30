/* eslint-disable react-hooks/refs -- useAudioInput returns a controller object
   containing the audio element ref alongside reactive selection state. */
import { useEffect, useState } from "react"

import type { Backend } from "@/api/backend"
import { AudioPlayer } from "@/components/demo/stages/AudioPlayer"
import { Button } from "@/components/ui/button"
import { useAudioInput } from "@/hooks/useAudioInput"

/** Plays uploaded audio without making it the next submission's selected file. */
export function JobAudio({
  backend,
  jobId,
  fileName,
}: {
  backend: Backend
  jobId: string
  fileName: string
}) {
  const audio = useAudioInput()
  const { selectFile } = audio
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading")
  const [attempt, setAttempt] = useState(0)

  // biome-ignore lint/correctness/useExhaustiveDependencies: Retrying increments attempt to deliberately restart this preload effect.
  useEffect(() => {
    let cancelled = false

    void backend
      .getJobAudio(jobId)
      .then((blob) => {
        if (cancelled) return
        selectFile(
          new File([blob], fileName, { type: blob.type || "audio/mpeg" })
        )
        setStatus("ready")
      })
      .catch(() => {
        if (!cancelled) setStatus("error")
      })

    return () => {
      cancelled = true
    }
  }, [attempt, backend, fileName, jobId, selectFile])

  return (
    <>
      {/* biome-ignore lint/a11y/useMediaCaption: This hidden media element is controlled by the adjacent custom player; the transcription stage renders its transcript. */}
      <audio
        ref={audio.audioRef}
        src={audio.url ?? undefined}
        className="hidden"
        preload="metadata"
      />
      {status === "loading" ? (
        <p role="status" className="text-sm text-muted-foreground">
          Loading audio…
        </p>
      ) : status === "error" ? (
        <div className="flex flex-col items-start gap-3">
          <p role="alert" className="text-sm text-destructive">
            The audio artifact is unavailable. Try loading it again.
          </p>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setStatus("loading")
              setAttempt((value) => value + 1)
            }}
          >
            Retry audio
          </Button>
        </div>
      ) : (
        <AudioPlayer audio={audio} removable={false} />
      )}
    </>
  )
}
