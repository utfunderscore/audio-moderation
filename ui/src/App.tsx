/* eslint-disable react-hooks/refs -- the audio and run hooks return controller
   objects that hold a ref field; the compiler rule cannot distinguish reading
   their state fields from reading the ref during render. */

import { Microphone01, MusicNote01, ShieldTick } from "@untitledui/icons"
import { useCallback, useEffect, useRef, useState } from "react"
import { toast } from "sonner"
import type { Backend } from "@/api/backend"
import { ElapsedDuration } from "@/components/demo/ElapsedDuration"
import { Header } from "@/components/demo/Header"
import { HistoricalJobDetails } from "@/components/demo/HistoricalJobDetails"
import { JobDetailsSidebar } from "@/components/demo/JobDetailsSidebar"
import { JobHistory } from "@/components/demo/JobHistory"
import { PipelineTimeline } from "@/components/demo/PipelineTimeline"
import { StagePage } from "@/components/demo/StagePage"
import { AudioPicker } from "@/components/demo/stages/AudioPicker"
import { AudioPlayer } from "@/components/demo/stages/AudioPlayer"
import { AudioProcessing } from "@/components/demo/stages/AudioProcessing"
import { ModerationStage } from "@/components/demo/stages/ModerationStage"
import { TranscriptionStage } from "@/components/demo/stages/TranscriptionStage"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import type { TerminalEvent } from "@/domain/events"
import { type AudioProcessingJob, DEMO_USER_ID } from "@/domain/jobs"
import type { PipelineState } from "@/domain/reducer"
import type { StageId, StageState } from "@/domain/stages"
import { useAudioInput } from "@/hooks/useAudioInput"
import { useEvaluation } from "@/hooks/useEvaluation"
import { useJobHistory } from "@/hooks/useJobHistory"
import { useMediaQuery } from "@/hooks/useMediaQuery"

type PageId = "audio" | "transcription" | "moderation"

const DESKTOP_VIEWPORT_QUERY = "(min-width: 1024px)"
const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
].join(",")

/**
 * The audio page covers ingress and conversion, so it is active as soon as the
 * evaluation is accepted, before stitched-audio events arrive.
 */
function audioPageState(state: PipelineState): StageState {
  if (state.stages.submitted === "failed") return "failed"
  if (
    state.stages.conversion === "pending" &&
    state.stages.submitted === "complete"
  ) {
    return "processing"
  }
  return state.stages.conversion
}

export function App({ backend }: { backend: Backend }) {
  const audio = useAudioInput()
  const evaluation = useEvaluation(backend)
  const isMobileViewport = !useMediaQuery(DESKTOP_VIEWPORT_QUERY)
  const {
    jobs: previousJobs,
    loading: jobsLoading,
    error: jobsError,
    refresh: refreshJobs,
  } = useJobHistory(backend, DEMO_USER_ID)
  const lastOutcome = useRef<TerminalEvent | null>(null)
  const [currentJobSelected, setCurrentJobSelected] = useState(false)
  const [selectedJobId, setSelectedJobId] = useState<string | null>(null)
  const [detailsSelectionKey, setDetailsSelectionKey] = useState(0)
  const detailsPanelRef = useRef<HTMLElement>(null)
  const detailsContentRef = useRef<HTMLDivElement>(null)
  const closeButtonRef = useRef<HTMLButtonElement>(null)
  const detailsInvokerRef = useRef<HTMLElement | null>(null)
  const currentAudioRef = audio.audioRef

  const pausePlayback = useCallback(() => {
    currentAudioRef.current?.pause()
    detailsPanelRef.current
      ?.querySelectorAll<HTMLAudioElement>("audio")
      .forEach((element) => {
        element.pause()
      })
  }, [currentAudioRef])

  const { state } = evaluation
  const { evaluationId, outcome } = state
  const historyVersion =
    evaluationId === null
      ? null
      : `${evaluationId}:${outcome ?? "processing"}:${evaluation.result === null ? "no-result" : "result"}`

  useEffect(() => {
    if (outcome === null) {
      lastOutcome.current = null
      return
    }
    if (lastOutcome.current === outcome) return
    lastOutcome.current = outcome
    if (outcome === "SUCCEEDED") {
      toast.success("Job completed")
    } else if (outcome === "FAILED") {
      toast.error("Job failed")
    } else {
      toast.info(`Job ${outcome.toLowerCase()}`)
    }
  }, [outcome])

  // The browser-local backend index changes when a run starts and settles.
  useEffect(() => {
    if (historyVersion === null) return
    void refreshJobs()
  }, [historyVersion, refreshJobs])

  const transcript =
    state.stages.transcription === "complete"
      ? evaluation.result?.transcript
      : undefined
  const scores =
    state.stages.moderation === "complete"
      ? evaluation.result?.scores
      : undefined

  const isReached = (stage: StageId) => {
    const value = state.stages[stage]
    return value !== "pending" && value !== "skipped"
  }

  const pageVisibility: Array<{ id: PageId; visible: boolean }> = [
    { id: "audio", visible: true },
    { id: "transcription", visible: isReached("transcription") },
    { id: "moderation", visible: isReached("moderation") },
  ]
  const visiblePages = pageVisibility
    .filter((page) => page.visible)
    .map((page) => page.id)
  const isLast = (id: PageId) => visiblePages[visiblePages.length - 1] === id

  /** Choosing audio starts the evaluation immediately. */
  const startRun = (file: File) => {
    pausePlayback()
    const activeElement = document.activeElement
    if (activeElement instanceof HTMLElement) {
      detailsInvokerRef.current = activeElement
    }
    setCurrentJobSelected(true)
    setSelectedJobId(null)
    setDetailsSelectionKey((key) => key + 1)
    evaluation.start({ userId: DEMO_USER_ID, audio: file })
  }

  const handleRemoveAudio = () => {
    evaluation.reset()
    audio.clear()
  }

  const audioStatus = audioPageState(state)
  const currentJobStatus: "processing" | "complete" | "failed" =
    evaluation.running
      ? "processing"
      : state.outcome === "SUCCEEDED"
        ? "complete"
        : state.outcome === null
          ? "processing"
          : "failed"
  const currentJob =
    audio.file !== null && evaluation.hasRun && state.startedAt !== null
      ? {
          id: state.evaluationId ?? "—",
          fileName: audio.file.name,
          startedAt: state.startedAt,
          endedAt: state.stageTimes.result.endedAt,
          status: currentJobStatus,
          scores,
        }
      : null
  const selectedJob =
    selectedJobId === null
      ? null
      : (previousJobs.find((job) => job.id === selectedJobId) ?? null)
  const sidebarVisible = currentJobSelected || selectedJob !== null

  const closeSidebar = useCallback(() => {
    pausePlayback()
    setCurrentJobSelected(false)
    setSelectedJobId(null)
  }, [pausePlayback])

  const openCurrentJob = () => {
    pausePlayback()
    const activeElement = document.activeElement
    if (activeElement instanceof HTMLElement) {
      detailsInvokerRef.current = activeElement
    }
    setCurrentJobSelected(true)
    setSelectedJobId(null)
    setDetailsSelectionKey((key) => key + 1)
  }

  const openHistoricalJob = (job: AudioProcessingJob) => {
    pausePlayback()
    const activeElement = document.activeElement
    if (activeElement instanceof HTMLElement) {
      detailsInvokerRef.current = activeElement
    }
    setCurrentJobSelected(false)
    setSelectedJobId(job.id)
    setDetailsSelectionKey((key) => key + 1)
  }

  useEffect(() => {
    if (!sidebarVisible || !isMobileViewport) return

    const scrollY = window.scrollY
    const bodyStyle = document.body.style
    const htmlStyle = document.documentElement.style
    const previousBodyStyles = {
      left: bodyStyle.left,
      overflow: bodyStyle.overflow,
      position: bodyStyle.position,
      right: bodyStyle.right,
      top: bodyStyle.top,
      width: bodyStyle.width,
    }
    const previousHtmlOverflow = htmlStyle.overflow

    bodyStyle.left = "0"
    bodyStyle.overflow = "hidden"
    bodyStyle.position = "fixed"
    bodyStyle.right = "0"
    bodyStyle.top = `-${scrollY}px`
    bodyStyle.width = "100%"
    htmlStyle.overflow = "hidden"

    return () => {
      bodyStyle.left = previousBodyStyles.left
      bodyStyle.overflow = previousBodyStyles.overflow
      bodyStyle.position = previousBodyStyles.position
      bodyStyle.right = previousBodyStyles.right
      bodyStyle.top = previousBodyStyles.top
      bodyStyle.width = previousBodyStyles.width
      htmlStyle.overflow = previousHtmlOverflow
      window.scrollTo(0, scrollY)
    }
  }, [isMobileViewport, sidebarVisible])

  // biome-ignore lint/correctness/useExhaustiveDependencies: The selection key intentionally triggers scrolling when a different job is selected.
  useEffect(() => {
    if (!sidebarVisible) return
    detailsContentRef.current?.scrollTo({ top: 0 })
  }, [detailsSelectionKey, sidebarVisible])

  useEffect(() => {
    if (!sidebarVisible || !isMobileViewport) return

    closeButtonRef.current?.focus()

    return () => {
      const invoker = detailsInvokerRef.current
      if (invoker?.isConnected) {
        invoker.focus({ preventScroll: true })
      }
    }
  }, [isMobileViewport, sidebarVisible])

  useEffect(() => {
    if (!sidebarVisible || !isMobileViewport) return

    const handleKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault()
        closeSidebar()
        return
      }
      if (event.key !== "Tab") return

      const panel = detailsPanelRef.current
      if (panel === null) return
      const focusableElements = Array.from(
        panel.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)
      ).filter((element) => element.getClientRects().length > 0)
      const firstElement = focusableElements[0]
      const lastElement = focusableElements.at(-1)

      if (firstElement === undefined || lastElement === undefined) return
      if (event.shiftKey && document.activeElement === firstElement) {
        event.preventDefault()
        lastElement.focus()
      } else if (!event.shiftKey && document.activeElement === lastElement) {
        event.preventDefault()
        firstElement.focus()
      } else if (!panel.contains(document.activeElement)) {
        event.preventDefault()
        const target = event.shiftKey ? lastElement : firstElement
        target.focus()
      }
    }

    document.addEventListener("keydown", handleKeyDown)
    return () => document.removeEventListener("keydown", handleKeyDown)
  }, [closeSidebar, isMobileViewport, sidebarVisible])

  return (
    <TooltipProvider>
      {/* biome-ignore lint/a11y/useMediaCaption: This hidden media element is controlled by the adjacent custom player; transcripts are rendered with each job. */}
      <audio
        ref={audio.audioRef}
        src={audio.url ?? undefined}
        className="hidden"
        preload="metadata"
      />
      <div className="flex min-h-svh flex-col bg-background">
        <Header />
        <div className="flex min-h-0 flex-1">
          <main className="min-w-0 flex-1 overflow-y-auto">
            <div className="mx-auto w-full max-w-5xl px-4 py-8 md:px-8 md:py-10">
              <div className="mb-8 text-center">
                <h1 className="text-2xl font-medium tracking-tight sm:text-3xl">
                  Automate moderation with{" "}
                  <span className="font-socialguard font-semibold tracking-normal">
                    socialguard
                  </span>
                </h1>
              </div>

              <section aria-label="Upload audio" className="mb-10">
                <AudioPicker
                  audio={audio}
                  disabled={evaluation.running}
                  onSelected={startRun}
                  title={
                    evaluation.running
                      ? "A job is currently processing"
                      : "Drop or paste audio here"
                  }
                  description={
                    evaluation.running
                      ? "Wait for it to finish before starting another"
                      : "MP3, WAV, M4A, AAC, OGG, WebM • 1 audio file"
                  }
                />
              </section>

              <JobHistory
                currentJob={currentJob}
                jobs={previousJobs}
                loading={jobsLoading}
                error={jobsError}
                selectedJobId={currentJobSelected ? null : selectedJobId}
                onOpenCurrent={openCurrentJob}
                onSelectJob={openHistoricalJob}
              />
            </div>
          </main>

          {sidebarVisible ? (
            <JobDetailsSidebar
              panelRef={detailsPanelRef}
              contentRef={detailsContentRef}
              closeButtonRef={closeButtonRef}
              selectedJob={selectedJob}
              audioFileName={audio.file?.name}
              isMobileViewport={isMobileViewport}
              onClose={closeSidebar}
            >
              {selectedJob !== null ? (
                <HistoricalJobDetails
                  key={selectedJob.id}
                  backend={backend}
                  job={selectedJob}
                />
              ) : (
                <PipelineTimeline>
                  <StagePage
                    title="Audio"
                    icon={<MusicNote01 aria-hidden />}
                    status={audioStatus}
                    elapsed={null}
                    elapsedContent={
                      <ElapsedDuration
                        startedAt={
                          state.stageTimes.conversion.startedAt ??
                          state.startedAt
                        }
                        endedAt={
                          state.stageTimes.conversion.endedAt ??
                          state.stageTimes.result.endedAt
                        }
                      />
                    }
                    isLast={isLast("audio")}
                  >
                    {audio.file === null ? (
                      <p className="rounded-lg border bg-muted/30 px-3 py-2 text-sm text-muted-foreground">
                        The selected audio is no longer available.
                      </p>
                    ) : audioStatus === "complete" ? (
                      <AudioPlayer
                        audio={audio}
                        disabled={evaluation.running}
                        onRemove={handleRemoveAudio}
                      />
                    ) : audioStatus === "failed" ? (
                      <p className="rounded-lg border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive">
                        Processing stopped before the audio was ready.
                      </p>
                    ) : (
                      <AudioProcessing fileName={audio.file.name} />
                    )}
                  </StagePage>

                  {isReached("transcription") ? (
                    <StagePage
                      title="Transcription"
                      icon={<Microphone01 aria-hidden />}
                      status={state.stages.transcription}
                      elapsed={null}
                      elapsedContent={
                        <ElapsedDuration
                          startedAt={state.stageTimes.transcription.startedAt}
                          endedAt={state.stageTimes.transcription.endedAt}
                        />
                      }
                      isLast={isLast("transcription")}
                    >
                      <TranscriptionStage
                        state={state.stages.transcription}
                        transcript={transcript}
                      />
                    </StagePage>
                  ) : null}

                  {isReached("moderation") ? (
                    <StagePage
                      title="Moderation"
                      icon={<ShieldTick aria-hidden />}
                      status={state.stages.moderation}
                      elapsed={null}
                      elapsedContent={
                        <ElapsedDuration
                          startedAt={state.stageTimes.moderation.startedAt}
                          endedAt={state.stageTimes.moderation.endedAt}
                        />
                      }
                      isLast={isLast("moderation")}
                    >
                      <ModerationStage
                        state={state.stages.moderation}
                        scores={scores}
                      />
                    </StagePage>
                  ) : null}
                </PipelineTimeline>
              )}
            </JobDetailsSidebar>
          ) : null}
        </div>
      </div>
      <Toaster position="bottom-right" />
    </TooltipProvider>
  )
}

export default App
