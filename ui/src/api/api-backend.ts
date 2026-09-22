import {
  type Client,
  ConnectError,
  createClient,
  type Interceptor,
  type Transport,
} from "@connectrpc/connect"
import { createConnectTransport } from "@connectrpc/connect-web"
import type {
  Backend,
  EvaluationResult,
  StartEvaluationInput,
  StartedEvaluation,
  TaskEventHandlers,
  Unsubscribe,
} from "@/api/backend"
import { BrowserEvaluationStore } from "@/api/browser-evaluation-store"
import { subscribeTaskEventsWebSocket } from "@/api/task-events-websocket"
import type { AudioProcessingJob } from "@/domain/jobs"
import {
  AudioModerationService,
  PipelineTaskStatus,
} from "@/gen/audio/moderation/v1/audio_moderation_pb"
import { AudioReviewService } from "@/gen/audio/review/v1/audio_review_pb"

const PENDING_STATUS = "PIPELINE_TASK_STATUS_PENDING"

const debugRpc: Interceptor = (next) => async (request) => {
  const method = `${request.service.typeName}/${request.method.name}`
  const startedAt = performance.now()
  console.debug("[RPC] request", method)
  try {
    const response = await next(request)
    console.debug("[RPC] response", method, {
      durationMs: Math.round(performance.now() - startedAt),
    })
    return response
  } catch (error) {
    console.debug("[RPC] failed", method, {
      durationMs: Math.round(performance.now() - startedAt),
      code: ConnectError.from(error).code,
      aborted: request.signal.aborted,
    })
    throw error
  }
}

interface EvaluationAccess {
  reviewId: string
  token: string
}

function reviewToken(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(32))
  let binary = ""
  for (const byte of bytes) binary += String.fromCharCode(byte)

  return `review_v1.${btoa(binary)
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replace(/=+$/, "")}`
}

async function responseError(
  operation: string,
  response: Response
): Promise<Error> {
  const detail = (await response.text()).trim()
  return new Error(
    `${operation} failed (${response.status})${detail === "" ? "" : `: ${detail}`}`
  )
}

/** Backend implementation for the application's HTTP and event-stream APIs. */
export class ApiBackend implements Backend {
  private readonly evaluationAccess = new Map<string, EvaluationAccess>()
  private readonly evaluationAudio = new Map<string, Blob>()
  private readonly evaluations: BrowserEvaluationStore
  private readonly moderationClient: Client<typeof AudioModerationService>
  private readonly reviewClient: Client<typeof AudioReviewService>
  private readonly taskEventsEndpoint: string

  constructor(
    endpoint = import.meta.env.VITE_API_ENDPOINT ?? "",
    transport: Transport = createConnectTransport({
      baseUrl: endpoint || "/",
      interceptors: [debugRpc],
    }),
    evaluations = new BrowserEvaluationStore(),
    taskEventsEndpoint = import.meta.env.VITE_TASK_EVENTS_ENDPOINT ?? ""
  ) {
    this.moderationClient = createClient(AudioModerationService, transport)
    this.reviewClient = createClient(AudioReviewService, transport)
    this.evaluations = evaluations
    this.taskEventsEndpoint = taskEventsEndpoint
  }

  async listJobs(userId: string): Promise<AudioProcessingJob[]> {
    return this.evaluations.listJobs(userId)
  }

  async startEvaluation(
    input: StartEvaluationInput
  ): Promise<StartedEvaluation> {
    const token = reviewToken()
    const submitted = await this.reviewClient.submitReview(
      {
        contentType: input.audio.type || "audio/octet-stream",
      },
      {
        headers: {
          authorization: `Bearer ${token}`,
          "idempotency-key": crypto.randomUUID(),
        },
      }
    )

    if (
      submitted.evaluationId === "" ||
      submitted.reviewId === "" ||
      submitted.uploadUrl === ""
    ) {
      throw new Error("SubmitReview returned an invalid upload response")
    }

    const upload = await fetch(submitted.uploadUrl, {
      method: "PUT",
      headers: submitted.uploadHeaders,
      body: input.audio,
    })
    if (!upload.ok) throw await responseError("Audio upload", upload)

    this.evaluationAccess.set(submitted.evaluationId, {
      reviewId: submitted.reviewId,
      token,
    })
    this.evaluationAudio.set(submitted.evaluationId, input.audio)
    this.evaluations.recordStarted({
      evaluationId: submitted.evaluationId,
      userId: input.userId,
      fileName: input.audio.name,
      submittedAt: Date.now(),
    })

    return {
      evaluationId: submitted.evaluationId,
      status: PENDING_STATUS,
    }
  }

  async getEvaluationResult(
    evaluationId: string
  ): Promise<EvaluationResult | null> {
    const access = this.evaluationAccess.get(evaluationId)
    if (access === undefined) return this.evaluations.getResult(evaluationId)

    const response = await this.moderationClient.getEvaluation(
      { evaluationId },
      { headers: { authorization: `Bearer ${access.token}` } }
    )
    const evaluation = response.evaluation
    if (evaluation === undefined) return null

    if (evaluation.status === PipelineTaskStatus.SUCCEEDED) {
      this.evaluations.recordFinished(evaluationId, "complete")
    } else if (
      evaluation.status === PipelineTaskStatus.FAILED ||
      evaluation.status === PipelineTaskStatus.TIMED_OUT ||
      evaluation.status === PipelineTaskStatus.CANCELLED
    ) {
      this.evaluations.recordFinished(evaluationId, "failed")
    }

    const transcript = evaluation.transcription?.transcript
    const remoteScores = evaluation.moderation?.scores
    const result: EvaluationResult = {
      transcript,
      scores:
        remoteScores === undefined
          ? undefined
          : {
              sexual: remoteScores.sexual,
              hate_or_discrimination: remoteScores.hateOrDiscrimination,
              harassment_or_abuse: remoteScores.harassmentOrAbuse,
              violence_or_threats: remoteScores.violenceOrThreats,
              asking_for_pii: remoteScores.askingForPii,
            },
    }
    if (result.transcript === undefined && result.scores === undefined) {
      return null
    }

    this.evaluations.recordResult(evaluationId, result)
    return result
  }

  async getJobAudio(jobId: string): Promise<Blob> {
    const audio = this.evaluationAudio.get(jobId)
    if (audio === undefined) {
      throw new Error(
        "Audio is only available in the tab that submitted this job"
      )
    }
    return audio
  }

  subscribeTaskEvents(
    evaluationId: string,
    handlers: TaskEventHandlers
  ): Unsubscribe {
    return subscribeTaskEventsWebSocket(
      this.taskEventsEndpoint,
      async (signal) => {
        const access = this.evaluationAccess.get(evaluationId)
        if (access === undefined) {
          throw new Error(
            "Evaluation access is only available in the tab that submitted this job"
          )
        }
        const { ticket } = await this.moderationClient.createTaskEventsTicket(
          { evaluationId },
          {
            headers: { authorization: `Bearer ${access.token}` },
            signal,
          }
        )
        return ticket
      },
      handlers
    )
  }
}
