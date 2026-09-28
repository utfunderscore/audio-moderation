// @vitest-environment jsdom
import { act, renderHook, waitFor } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"
import type { Backend, TaskEventHandlers } from "@/api/backend"
import { useEvaluation } from "./useEvaluation"

describe("useEvaluation", () => {
  it("shows the persisted transcript after ASR finishes, while moderation is running", async () => {
    let handlers: TaskEventHandlers | undefined
    const getEvaluationResult = vi.fn().mockResolvedValue({
      transcript: "the spoken words",
    })
    const backend = {
      startEvaluation: vi.fn().mockResolvedValue({
        evaluationId: "evaluation-1",
        status: "PIPELINE_TASK_STATUS_PENDING",
      }),
      subscribeTaskEvents: vi.fn((_id: string, next: TaskEventHandlers) => {
        handlers = next
        return () => {}
      }),
      getEvaluationResult,
    } as unknown as Backend
    const { result } = renderHook(() => useEvaluation(backend))

    act(() => {
      result.current.start({
        userId: "user-1",
        audio: new File(["audio"], "sample.mp3", { type: "audio/mpeg" }),
      })
    })
    await waitFor(() => expect(handlers).toBeDefined())
    act(() => {
      handlers?.onFrame({ name: "ASR_STARTED" })
      handlers?.onFrame({ name: "ASR_FINISHED" })
      handlers?.onFrame({ name: "MODERATION_PROCESSING_STARTED" })
    })

    await waitFor(() =>
      expect(result.current.result?.transcript).toBe("the spoken words")
    )
    expect(result.current.state.outcome).toBeNull()
    expect(result.current.state.stages.transcription).toBe("complete")
    expect(result.current.state.stages.moderation).toBe("processing")
    expect(getEvaluationResult).toHaveBeenCalledWith("evaluation-1")
  })

  it("reads a transcript for a resumed run already past ASR, then refreshes scores at completion", async () => {
    let handlers: TaskEventHandlers | undefined
    const getEvaluationResult = vi
      .fn()
      .mockResolvedValueOnce({ transcript: "early transcript" })
      .mockResolvedValueOnce({
        transcript: "early transcript",
        scores: { sexual: 0.01 },
      })
    const backend = {
      resumeEvaluation: vi.fn().mockResolvedValue({
        evaluationId: "evaluation-1",
        status: "PIPELINE_TASK_STATUS_STARTED_MODERATION_PROCESSING",
      }),
      subscribeTaskEvents: vi.fn((_id: string, next: TaskEventHandlers) => {
        handlers = next
        return () => {}
      }),
      getEvaluationResult,
    } as unknown as Backend
    const { result } = renderHook(() => useEvaluation(backend))

    act(() =>
      result.current.resume({ evaluationId: "evaluation-1", startedAt: 1_000 })
    )
    await waitFor(() =>
      expect(result.current.result?.transcript).toBe("early transcript")
    )
    expect(result.current.state.outcome).toBeNull()
    act(() => {
      handlers?.onFrame({ name: "ASR_FINISHED" }) // replayed frame
    })
    expect(getEvaluationResult).toHaveBeenCalledTimes(1)
    act(() => {
      handlers?.onFrame({ name: "SUCCEEDED" })
    })
    await waitFor(() =>
      expect(result.current.result?.scores?.sexual).toBe(0.01)
    )
    expect(getEvaluationResult).toHaveBeenCalledTimes(2)
  })
})
