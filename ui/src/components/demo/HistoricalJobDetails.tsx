/* eslint-disable react-hooks/refs -- useAudioInput returns a controller object
   containing the audio element ref alongside reactive selection state. */
import { Microphone01, MusicNote01, ShieldTick } from "@untitledui/icons"
import { useEffect, useState } from "react"

import { PipelineTimeline } from "@/components/demo/PipelineTimeline"
import { StagePage } from "@/components/demo/StagePage"
import { AudioPlayer } from "@/components/demo/stages/AudioPlayer"
import { ModerationStage } from "@/components/demo/stages/ModerationStage"
import { TranscriptionStage } from "@/components/demo/stages/TranscriptionStage"
import type { AudioProcessingJob } from "@/domain/jobs"
import { useAudioInput } from "@/hooks/useAudioInput"
import { preloadDemoAudioArtifact } from "@/transport/audioArtifacts"
import { Button } from "@/components/ui/button"

function HistoricalAudio({ fileName }: { fileName: string }) {
  const audio = useAudioInput()
  const { selectFile } = audio
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading")
  const [attempt, setAttempt] = useState(0)

  useEffect(() => {
    let cancelled = false

    void preloadDemoAudioArtifact()
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
  }, [attempt, fileName, selectFile])

  return (
    <>
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

export function HistoricalJobDetails({ job }: { job: AudioProcessingJob }) {
  const complete = job.status === "complete"

  return (
    <>
      <PipelineTimeline>
        <StagePage
          title="Audio"
          icon={<MusicNote01 aria-hidden />}
          status={complete ? "complete" : "failed"}
          elapsed={complete ? 2_200 : null}
          isLast={!complete}
        >
          {complete ? (
            <HistoricalAudio key={job.fileName} fileName={job.fileName} />
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
    </>
  )
}
