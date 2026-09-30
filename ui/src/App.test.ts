// @vitest-environment jsdom

import { createRouterTransport } from "@connectrpc/connect"
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react"
import { createElement } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { App } from "@/App"
import { ApiBackend } from "@/api/api-backend"
import type { Backend } from "@/api/backend"
import { BrowserEvaluationAccessStore } from "@/api/browser-evaluation-access-store"
import { BrowserEvaluationStore } from "@/api/browser-evaluation-store"
import { ThemeProvider } from "@/components/theme-provider"
import { DEMO_USER_ID } from "@/domain/jobs"
import { PIPELINE_STATUS } from "@/domain/reducer"
import { createStageTimes } from "@/domain/stages"
import {
  AudioModerationService,
  PipelineTaskStatus,
} from "@/gen/audio/moderation/v1/audio_moderation_pb"

// jsdom has no audio decoder/canvas; keep the real player and input controller.
vi.mock("@/components/demo/Waveform", () => ({ Waveform: () => null }))

function reloadedBackend() {
  return {
    listJobs: vi.fn<Backend["listJobs"]>().mockResolvedValue([
      {
        id: "evaluation-1",
        userId: DEMO_USER_ID,
        fileName: "voice-note.wav",
        submittedAt: Date.now() - 10_000,
        durationMs: null,
        status: "processing",
      },
    ]),
    startEvaluation: vi.fn<Backend["startEvaluation"]>(),
    resumeEvaluation: vi.fn<Backend["resumeEvaluation"]>().mockResolvedValue({
      evaluationId: "evaluation-1",
      status: PIPELINE_STATUS.startedAsr,
      stageTimes: createStageTimes(),
    }),
    getEvaluationResult: vi
      .fn<Backend["getEvaluationResult"]>()
      .mockResolvedValue(null),
    getJobAudio: vi
      .fn<Backend["getJobAudio"]>()
      .mockResolvedValue(new Blob(["uploaded audio"], { type: "audio/wav" })),
    subscribeTaskEvents: vi
      .fn<Backend["subscribeTaskEvents"]>()
      .mockReturnValue(() => {}),
  } satisfies Backend
}

async function openCurrentJob(backend: Backend) {
  render(createElement(App, { backend }), { wrapper: ThemeProvider })
  await waitFor(() => {
    const row = screen.getByRole("row", {
      name: "Open job evaluation-1: voice-note.wav",
    })
    expect(row.getAttribute("aria-current")).toBe("true")
  })
  fireEvent.click(
    screen.getByRole("row", {
      name: "Open job evaluation-1: voice-note.wav",
    })
  )
  return screen.getByRole("complementary")
}

beforeEach(() => {
  vi.stubGlobal(
    "matchMedia",
    vi.fn((query: string) => ({
      matches:
        query === "(min-width: 1024px)" ||
        query === "(prefers-reduced-motion: reduce)",
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    }))
  )
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {})
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  )
  vi.stubGlobal(
    "URL",
    Object.assign(class extends URL {}, {
      createObjectURL: vi.fn(() => "blob:restored-audio"),
      revokeObjectURL: vi.fn(),
    })
  )
  Element.prototype.scrollTo = vi.fn()
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe("in-progress job after reload", () => {
  it.each([
    PipelineTaskStatus.STARTED_ASR,
    PipelineTaskStatus.STARTED_MODERATION_PROCESSING,
  ])(
    "preserves each stage's elapsed time instead of adding it to audio on reload (%s)",
    async (resumedStatus) => {
      let now = 1_000
      vi.spyOn(Date, "now").mockImplementation(() => now)
      const values = new Map<string, string>()
      const storage = {
        getItem: (key: string) => values.get(key) ?? null,
        setItem: (key: string, value: string) => {
          values.set(key, value)
        },
      }
      new BrowserEvaluationStore(storage).recordStarted({
        evaluationId: "evaluation-1",
        userId: DEMO_USER_ID,
        fileName: "voice-note.wav",
        submittedAt: now,
      })
      new BrowserEvaluationAccessStore(storage).set("evaluation-1", {
        reviewId: "review-1",
        token: "test-owner",
      })
      let status = PipelineTaskStatus.PENDING
      const transport = createRouterTransport(({ service }) => {
        service(AudioModerationService, {
          getEvaluation: () => ({
            evaluation: {
              evaluationId: "evaluation-1",
              status,
              createdAt: { seconds: 1n },
              ...(status !== PipelineTaskStatus.PENDING
                ? {
                    audioProcessing: {
                      startedAt: { seconds: 2n },
                      completedAt: { seconds: 4n },
                    },
                    transcription: {
                      startedAt: { seconds: 4n },
                      ...(status ===
                      PipelineTaskStatus.STARTED_MODERATION_PROCESSING
                        ? { completedAt: { seconds: 7n } }
                        : {}),
                    },
                    ...(status ===
                    PipelineTaskStatus.STARTED_MODERATION_PROCESSING
                      ? { moderation: { startedAt: { seconds: 7n } } }
                      : {}),
                  }
                : {}),
            },
          }),
        })
      })
      function loadPage() {
        const backend = new ApiBackend(
          "https://api.example",
          transport,
          new BrowserEvaluationStore(storage),
          "",
          new BrowserEvaluationAccessStore(storage)
        )
        const subscribe = vi
          .spyOn(backend, "subscribeTaskEvents")
          .mockReturnValue(() => {})
        vi.spyOn(backend, "getJobAudio").mockResolvedValue(new Blob(["audio"]))
        return { backend, subscribe }
      }
      const first = loadPage()
      await openCurrentJob(first.backend)
      const handlers = first.subscribe.mock.calls[0][1]
      now = 2_000
      act(() => handlers.onFrame({ name: "AUDIO_PROCESSING_STARTED" }))
      now = 4_000
      act(() => {
        handlers.onFrame({ name: "AUDIO_PROCESSING_FINISHED" })
        handlers.onFrame({ name: "ASR_STARTED" })
      })
      await screen.findByRole("button", { name: "Play" })
      now = 11_000
      status = resumedStatus
      cleanup()

      const reloaded = loadPage()
      const panel = await openCurrentJob(reloaded.backend)
      const audioStage = within(panel)
        .getByRole("heading", { name: "Audio" })
        .closest("li")
      const transcriptionStage = within(panel)
        .getByRole("heading", { name: "Transcription" })
        .closest("li")
      expect
        .soft(audioStage?.querySelector(".pipeline-status-enter")?.textContent)
        .toBe("3s")
      expect
        .soft(
          transcriptionStage?.querySelector(".pipeline-status-enter")
            ?.textContent
        )
        .toBe(resumedStatus === PipelineTaskStatus.STARTED_ASR ? "7s" : "3s")
      if (resumedStatus === PipelineTaskStatus.STARTED_MODERATION_PROCESSING) {
        const moderationStage = within(panel)
          .getByRole("heading", { name: "Moderation" })
          .closest("li")
        expect(
          moderationStage?.querySelector(".pipeline-status-enter")?.textContent
        ).toBe("4s")
      }
    }
  )

  it("shows unknown stage durations instead of inventing timestamps when the snapshot has none", async () => {
    const panel = await openCurrentJob(reloadedBackend())
    const audioStage = within(panel)
      .getByRole("heading", { name: "Audio" })
      .closest("li")
    const transcriptionStage = within(panel)
      .getByRole("heading", { name: "Transcription" })
      .closest("li")
    expect(
      audioStage?.querySelector(".pipeline-status-enter")?.textContent
    ).toBe("—")
    expect(
      transcriptionStage?.querySelector(".pipeline-status-enter")?.textContent
    ).toBe("—")
  })

  it("restores the current job's filename in the details header", async () => {
    const panel = await openCurrentJob(reloadedBackend())
    expect(panel.textContent).not.toContain("No job selected")
    const header = within(panel).getByRole("heading", {
      name: "Current job",
    }).parentElement
    expect(header?.textContent).toContain("voice-note.wav")
  })

  it("reloads the uploaded audio for the current job's player", async () => {
    const backend = reloadedBackend()
    const panel = await openCurrentJob(backend)
    await waitFor(() =>
      expect(backend.getJobAudio).toHaveBeenCalledWith("evaluation-1")
    )
    expect(
      await within(panel).findByRole("button", { name: "Play" })
    ).toBeDefined()
    expect(panel.textContent).not.toContain(
      "The selected audio is no longer available"
    )
    expect(backend.startEvaluation).not.toHaveBeenCalled()
    expect(
      screen.queryByRole("button", { name: "Verify and submit audio" })
    ).toBeNull()
  })

  it("keeps the filename visible while audio conversion is still running", async () => {
    const backend = reloadedBackend()
    backend.resumeEvaluation.mockResolvedValue({
      evaluationId: "evaluation-1",
      status: PIPELINE_STATUS.startedAudioProcessing,
    })
    const panel = await openCurrentJob(backend)
    expect(panel.textContent).toContain("voice-note.wav")
    expect(panel.textContent).not.toContain(
      "The selected audio is no longer available"
    )
    expect(backend.getJobAudio).not.toHaveBeenCalled()
  })

  it("allows retrying a failed audio download without interrupting the pipeline", async () => {
    const backend = reloadedBackend()
    backend.getJobAudio.mockRejectedValueOnce(new Error("temporary failure"))
    const panel = await openCurrentJob(backend)
    fireEvent.click(
      await within(panel).findByRole("button", { name: "Retry audio" })
    )
    expect(
      await within(panel).findByRole("button", { name: "Play" })
    ).toBeDefined()
    expect(backend.getJobAudio).toHaveBeenCalledTimes(2)
    expect(backend.resumeEvaluation).toHaveBeenCalledTimes(1)
    expect(backend.subscribeTaskEvents).toHaveBeenCalledTimes(1)
  })

  it("loads audio when conversion finishes after the job has resumed", async () => {
    const backend = reloadedBackend()
    backend.resumeEvaluation.mockResolvedValue({
      evaluationId: "evaluation-1",
      status: PIPELINE_STATUS.startedAudioProcessing,
    })
    const panel = await openCurrentJob(backend)
    const handlers = backend.subscribeTaskEvents.mock.calls[0][1]
    act(() => handlers.onFrame({ name: "AUDIO_PROCESSING_FINISHED" }))
    expect(
      await within(panel).findByRole("button", { name: "Play" })
    ).toBeDefined()
    expect(backend.getJobAudio).toHaveBeenCalledWith("evaluation-1")
  })

  it("ignores an audio download that finishes after the page unmounts", async () => {
    const backend = reloadedBackend()
    let finishDownload!: (audio: Blob) => void
    backend.getJobAudio.mockReturnValue(
      new Promise((resolve) => {
        finishDownload = resolve
      })
    )
    const panel = await openCurrentJob(backend)
    expect(within(panel).getByText("Loading audio…")).toBeDefined()
    cleanup()
    await act(async () => finishDownload(new Blob(["late audio"])))
    expect(URL.createObjectURL).not.toHaveBeenCalled()
  })

  it("releases the restored audio URL when the page unmounts", async () => {
    const panel = await openCurrentJob(reloadedBackend())
    await within(panel).findByRole("button", { name: "Play" })
    cleanup()
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:restored-audio")
  })
})
