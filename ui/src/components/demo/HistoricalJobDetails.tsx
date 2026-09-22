/* eslint-disable react-hooks/refs -- useAudioInput returns a controller object
   containing the audio element ref alongside reactive selection state. */
import { Microphone01, MusicNote01, ShieldTick } from "@untitledui/icons"
import { useEffect, useState } from "react"

import type { Backend } from "@/api/backend"
import { PipelineTimeline } from "@/components/demo/PipelineTimeline"
import { StagePage } from "@/components/demo/StagePage"
import { AudioPlayer } from "@/components/demo/stages/AudioPlayer"
import { ModerationStage } from "@/components/demo/stages/ModerationStage"
import { TranscriptionStage } from "@/components/demo/stages/TranscriptionStage"
import { Button } from "@/components/ui/button"
import type { AudioProcessingJob } from "@/domain/jobs"
import { useAudioInput } from "@/hooks/useAudioInput"

function HistoricalAudio({
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

export function HistoricalJobDetails({
  backend,
  job,
}: {
  backend: Backend
  job: AudioProcessingJob
}) {
  const complete = job.status === "complete"
  const processing = job.status === "processing"

  return (
    <PipelineTimeline>
      <StagePage
        title="Audio"
        icon={<MusicNote01 aria-hidden />}
        status={complete ? "complete" : processing ? "processing" : "failed"}
        elapsed={complete ? 2_200 : null}
        isLast={!complete}
      >
        {complete ? (
          <HistoricalAudio
            key={job.fileName}
            backend={backend}
            jobId={job.id}
            fileName={job.fileName}
          />
        ) : processing ? (
          <p className="rounded-lg border bg-muted/30 px-3 py-2 text-sm text-muted-foreground">
            This evaluation is still processing.
          </p>
        ) : (
          <p className="rounded-lg border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive">
            Processing stopped before the audio was ready.
          </p>
        )}
      </StagePage>

      {complete ? (
        <StagePage
          title="Transcription"
          icon={<Microphone01 aria-hidden />}
          status="complete"
          elapsed={4_400}
        >
          <TranscriptionStage state="complete" transcript={job.transcript} />
        </StagePage>
      ) : null}

      {complete ? (
        <StagePage
          title="Moderation"
          icon={<ShieldTick aria-hidden />}
          status="complete"
          elapsed={4_200}
          isLast
        >
          <ModerationStage state="complete" scores={job.scores} />
        </StagePage>
      ) : null}
    </PipelineTimeline>
  )
}
