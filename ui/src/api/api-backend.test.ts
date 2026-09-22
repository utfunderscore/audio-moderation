import { Code, ConnectError, createRouterTransport } from "@connectrpc/connect"
import { afterEach, describe, expect, it, vi } from "vitest"
import {
  AudioModerationService,
  PipelineTaskStatus,
} from "@/gen/audio/moderation/v1/audio_moderation_pb"
import {
  AudioReviewService,
  ReviewJobStatus,
} from "@/gen/audio/review/v1/audio_review_pb"
import { ApiBackend } from "./api-backend"
import { BrowserEvaluationStore } from "./browser-evaluation-store"

class MemoryStorage {
  private readonly values = new Map<string, string>()

  getItem(key: string): string | null {
    return this.values.get(key) ?? null
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value)
  }
}

describe("ApiBackend.startEvaluation", () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it("creates an authorized review and uploads the selected audio", async () => {
    const audio = new File(["audio bytes"], "sample.wav", {
      type: "audio/wav",
    })
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValue(new Response(null, { status: 200 }))
    vi.stubGlobal("fetch", fetcher)
    const transport = createRouterTransport(({ service }) => {
      service(AudioReviewService, {
        submitReview(request, context) {
          expect(request.contentType).toBe("audio/wav")
          expect(context.requestHeader.get("authorization")).toMatch(
            /^Bearer review_v1.[A-Za-z0-9_-]{43}$/
          )
          expect(context.requestHeader.get("idempotency-key")).toMatch(
            /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
          )
          return {
            evaluationId: "evaluation-1",
            reviewId: "42",
            uploadUrl: "https://uploads.example/source",
            uploadHeaders: {
              "content-type": "audio/wav",
              "x-amz-meta-test": "signed",
            },
            status: ReviewJobStatus.AWAITING_UPLOAD,
          }
        },
      })
    })
    const backend = new ApiBackend("https://api.example/", transport)

    await expect(
      backend.startEvaluation({ userId: "user-1", audio })
    ).resolves.toEqual({
      evaluationId: "evaluation-1",
      status: "PIPELINE_TASK_STATUS_PENDING",
    })

    expect(fetcher).toHaveBeenCalledOnce()
    const [uploadUrl, uploadInit] = fetcher.mock.calls[0]
    expect(uploadUrl).toBe("https://uploads.example/source")
    expect(uploadInit?.method).toBe("PUT")
    expect(uploadInit?.body).toBe(audio)
    const headers = new Headers(uploadInit?.headers)
    expect(headers.get("content-type")).toBe("audio/wav")
    expect(headers.get("x-amz-meta-test")).toBe("signed")
  })

  it.each([false, true])(
    "subscribes with a ticket and supports early unsubscribe (%s)",
    async (cancelEarly) => {
      vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(null)))
      const socket = {
        send: vi.fn(),
        close: vi.fn(),
        onopen: null as (() => void) | null,
        onmessage: null as ((event: { data: string }) => void) | null,
      }
      // biome-ignore lint/complexity/useArrowFunction: WebSocket must be constructable.
      const WebSocketMock = vi.fn(function () {
        return socket
      })
      vi.stubGlobal("WebSocket", WebSocketMock)
      let releaseTicket!: (value: { ticket: string }) => void
      const ticketResponse = new Promise<{ ticket: string }>((resolve) => {
        releaseTicket = resolve
      })
      let authorization = ""
      const transport = createRouterTransport(({ service }) => {
        service(AudioReviewService, {
          submitReview(_request, context) {
            authorization = context.requestHeader.get("authorization") ?? ""
            return {
              evaluationId: "evaluation-1",
              reviewId: "42",
              uploadUrl: "https://uploads.example/source",
            }
          },
        })
        service(AudioModerationService, {
          createTaskEventsTicket(request, context) {
            expect(request.evaluationId).toBe("evaluation-1")
            expect(context.requestHeader.get("authorization")).toBe(
              authorization
            )
            return ticketResponse
          },
        })
      })
      const backend = new ApiBackend(
        "https://api.example",
        transport,
        new BrowserEvaluationStore(new MemoryStorage()),
        "wss://events.example/live"
      )
      await backend.startEvaluation({
        userId: "user-1",
        audio: new File(["audio"], "sample.wav"),
      })
      const handlers = {
        onFrame: vi.fn(),
        onConnectionChange: vi.fn(),
        onError: vi.fn(),
      }
      const unsubscribe = backend.subscribeTaskEvents("evaluation-1", handlers)
      if (cancelEarly) unsubscribe()
      releaseTicket({ ticket: "one-time-ticket" })
      if (cancelEarly) {
        await new Promise((resolve) => setTimeout(resolve, 0))
        expect(WebSocketMock).not.toHaveBeenCalled()
        expect(handlers.onError).not.toHaveBeenCalled()
        return
      }
      await vi.waitFor(() =>
        expect(WebSocketMock).toHaveBeenCalledWith("wss://events.example/live")
      )
      socket.onopen?.()
      expect(socket.send).toHaveBeenCalledWith(
        JSON.stringify({ action: "subscribe", ticket: "one-time-ticket" })
      )
      socket.onmessage?.({ data: "TASK_SUCCEEDED" })
      expect(handlers.onFrame).toHaveBeenCalledWith({ name: "TASK_SUCCEEDED" })
      unsubscribe()
      socket.onmessage?.({ data: "LATE_EVENT" })
      expect(handlers.onFrame).toHaveBeenCalledTimes(1)
      expect(socket.close).toHaveBeenCalledOnce()
    }
  )

  it("does not upload when SubmitReview fails", async () => {
    const fetcher = vi.fn<typeof fetch>()
    vi.stubGlobal("fetch", fetcher)
    const transport = createRouterTransport(({ service }) => {
      service(AudioReviewService, {
        submitReview() {
          throw new ConnectError("not authorized", Code.Unauthenticated)
        },
      })
    })
    const backend = new ApiBackend("https://api.example", transport)

    await expect(
      backend.startEvaluation({
        userId: "user-1",
        audio: new File(["audio"], "sample.mp3", { type: "audio/mpeg" }),
      })
    ).rejects.toThrow("not authorized")
    expect(fetcher).not.toHaveBeenCalled()
  })

  it("fetches and persists a submitted evaluation's terminal result", async () => {
    vi.stubGlobal(
      "fetch",
      vi
        .fn<typeof fetch>()
        .mockResolvedValue(new Response(null, { status: 200 }))
    )
    let evaluationRequests = 0
    const transport = createRouterTransport(({ service }) => {
      let reviewAuthorization = ""
      service(AudioReviewService, {
        submitReview(_request, context) {
          reviewAuthorization = context.requestHeader.get("authorization") ?? ""
          return {
            evaluationId: "evaluation-1",
            reviewId: "42",
            uploadUrl: "https://uploads.example/source",
            status: ReviewJobStatus.AWAITING_UPLOAD,
          }
        },
      })
      service(AudioModerationService, {
        getEvaluation(request, context) {
          evaluationRequests += 1
          expect(request.evaluationId).toBe("evaluation-1")
          expect(context.requestHeader.get("authorization")).toBe(
            reviewAuthorization
          )
          return {
            evaluation: {
              evaluationId: "evaluation-1",
              status: PipelineTaskStatus.SUCCEEDED,
              transcription: { transcript: "remote transcript" },
              moderation: {
                scores: {
                  sexual: 0.01,
                  hateOrDiscrimination: 0.02,
                  harassmentOrAbuse: 0.03,
                  violenceOrThreats: 0.04,
                  askingForPii: 0.05,
                },
              },
            },
          }
        },
      })
    })
    const storage = new MemoryStorage()
    const backend = new ApiBackend(
      "https://api.example",
      transport,
      new BrowserEvaluationStore(storage)
    )
    const audio = new File(["audio"], "voice-note.mp3", {
      type: "audio/mpeg",
    })

    await backend.startEvaluation({ userId: "user-1", audio })

    await expect(backend.listJobs("user-1")).resolves.toMatchObject([
      {
        id: "evaluation-1",
        userId: "user-1",
        fileName: "voice-note.mp3",
        status: "processing",
      },
    ])
    await expect(backend.getJobAudio("evaluation-1")).resolves.toBe(audio)

    await expect(backend.getEvaluationResult("evaluation-1")).resolves.toEqual({
      transcript: "remote transcript",
      scores: {
        sexual: 0.01,
        hate_or_discrimination: 0.02,
        harassment_or_abuse: 0.03,
        violence_or_threats: 0.04,
        asking_for_pii: 0.05,
      },
    })
    expect(evaluationRequests).toBe(1)
    await expect(backend.listJobs("user-1")).resolves.toMatchObject([
      {
        status: "complete",
        transcript: "remote transcript",
        scores: { asking_for_pii: 0.05 },
      },
    ])

    const restored = new BrowserEvaluationStore(storage)
    const restoredBackend = new ApiBackend(
      "https://api.example",
      transport,
      restored
    )
    await expect(
      restoredBackend.getEvaluationResult("evaluation-1")
    ).resolves.toMatchObject({
      transcript: "remote transcript",
      scores: { asking_for_pii: 0.05 },
    })
    expect(evaluationRequests).toBe(1)
  })
})
