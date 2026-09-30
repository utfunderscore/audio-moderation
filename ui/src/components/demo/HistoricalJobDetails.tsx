import { Microphone01, MusicNote01, ShieldTick } from "@untitledui/icons"

import type { Backend } from "@/api/backend"
import { JobAudio } from "@/components/demo/JobAudio"
import { PipelineTimeline } from "@/components/demo/PipelineTimeline"
import { StagePage } from "@/components/demo/StagePage"
import { ModerationStage } from "@/components/demo/stages/ModerationStage"
import { TranscriptionStage } from "@/components/demo/stages/TranscriptionStage"
import type { AudioProcessingJob } from "@/domain/jobs"

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
          <JobAudio
            key={job.id}
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
