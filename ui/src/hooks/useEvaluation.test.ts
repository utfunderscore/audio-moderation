// @vitest-environment jsdom

import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { Backend, TaskEventHandlers } from "@/api/backend"
import { PIPELINE_STATUS } from "@/domain/reducer"
import { useEvaluation } from "./useEvaluation"

afterEach(cleanup)

function setup(status: string) {
  let handlers: TaskEventHandlers
  const backend = {
    listJobs: vi.fn(),
    startEvaluation: vi.fn(),
    resumeEvaluation: vi.fn().mockResolvedValue({
      evaluationId: "evaluation-1",
      status,
    }),
    getEvaluationResult: vi
      .fn()
      .mockResolvedValue({ transcript: "Hello world." }),
    getJobAudio: vi.fn(),
    subscribeTaskEvents: vi.fn((_id, next: TaskEventHandlers) => {
      handlers = next
      return () => {}
    }),
  } satisfies Backend
  const hook = renderHook(() => useEvaluation(backend))
  act(() =>
    hook.result.current.resume({ evaluationId: "evaluation-1", startedAt: 1 })
  )
  return {
    ...hook,
    backend,
    frame(name: string) {
      act(() => handlers.onFrame({ name }))
    },
  }
}

describe("partial evaluation results", () => {
  it("fetches the transcript when transcription finishes, before moderation settles", async () => {
    const { result, backend, frame } = setup(PIPELINE_STATUS.startedAsr)
    await waitFor(() => expect(backend.subscribeTaskEvents).toHaveBeenCalled())
    expect(backend.getEvaluationResult).not.toHaveBeenCalled()
    frame("ASR_FINISHED")
    frame("MODERATION_PROCESSING_STARTED")
    await waitFor(() =>
      expect(result.current.result?.transcript).toBe("Hello world.")
    )
    expect(result.current.state.outcome).toBeNull()
    expect(result.current.state.stages.moderation).toBe("processing")

    backend.getEvaluationResult.mockResolvedValue({
      transcript: "Hello world.",
      scores: {
        sexual: 0,
        hate_or_discrimination: 0,
        harassment_or_abuse: 0,
        violence_or_threats: 0,
        asking_for_pii: 0,
      },
    })
    frame("MODERATION_PROCESSING_FINISHED")
    frame("SUCCEEDED")
    await waitFor(() => expect(result.current.result?.scores).toBeDefined())
  })

  it("fetches the available transcript when resuming during moderation", async () => {
    const { result } = setup(PIPELINE_STATUS.startedModerationProcessing)
    await waitFor(() =>
      expect(result.current.result?.transcript).toBe("Hello world.")
    )
    expect(result.current.state.outcome).toBeNull()
  })
})
