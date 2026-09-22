import type { EvaluationResult } from "@/api/backend"
import type { AudioJobStatus, AudioProcessingJob } from "@/domain/jobs"
import type { ModerationScores } from "@/domain/moderation"

const STORAGE_KEY = "audio-moderation.evaluations.v1"
const MAX_JOBS = 100

interface StoredEvaluations {
  version: 1
  jobs: AudioProcessingJob[]
}

export interface StartedEvaluationRecord {
  evaluationId: string
  userId: string
  fileName: string
  submittedAt: number
}

interface StorageLike {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
}

function isStatus(value: unknown): value is AudioJobStatus {
  return value === "processing" || value === "complete" || value === "failed"
}

function isScores(value: unknown): value is ModerationScores {
  if (typeof value !== "object" || value === null) return false
  const scores = value as Partial<ModerationScores>
  return (
    typeof scores.sexual === "number" &&
    typeof scores.hate_or_discrimination === "number" &&
    typeof scores.harassment_or_abuse === "number" &&
    typeof scores.violence_or_threats === "number" &&
    typeof scores.asking_for_pii === "number"
  )
}

function isJob(value: unknown): value is AudioProcessingJob {
  if (typeof value !== "object" || value === null) return false
  const job = value as Partial<AudioProcessingJob>
  return (
    typeof job.id === "string" &&
    typeof job.userId === "string" &&
    typeof job.fileName === "string" &&
    typeof job.submittedAt === "number" &&
    (typeof job.durationMs === "number" || job.durationMs === null) &&
    isStatus(job.status) &&
    (job.transcript === undefined || typeof job.transcript === "string") &&
    (job.scores === undefined || isScores(job.scores))
  )
}

function parseJobs(serialized: string | null): AudioProcessingJob[] {
  if (serialized === null) return []
  try {
    const value = JSON.parse(serialized) as Partial<StoredEvaluations>
    if (value.version !== 1 || !Array.isArray(value.jobs)) return []
    return value.jobs.filter(isJob)
  } catch {
    return []
  }
}

function defaultStorage(): StorageLike | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage
  } catch {
    // Some browser privacy modes expose localStorage but reject access to it.
    return undefined
  }
}

function copyJob(job: AudioProcessingJob): AudioProcessingJob {
  return {
    ...job,
    scores: job.scores === undefined ? undefined : { ...job.scores },
  }
}

/**
 * Small origin-local index of evaluations submitted by this browser.
 *
 * localStorage is intentional here: there is no server-side list endpoint and
 * job metadata/results are small JSON values that should survive a reload. The
 * implementation retains an in-memory fallback when storage is unavailable.
 */
export class BrowserEvaluationStore {
  private jobs: AudioProcessingJob[] = []
  private storage: StorageLike | undefined

  constructor(storage: StorageLike | undefined = defaultStorage()) {
    this.storage = storage
    this.jobs = this.readStoredJobs()
  }

  listJobs(userId: string): AudioProcessingJob[] {
    return this.readStoredJobs()
      .filter((job) => job.userId === userId)
      .sort((left, right) => right.submittedAt - left.submittedAt)
      .map(copyJob)
  }

  recordStarted(record: StartedEvaluationRecord): void {
    this.upsert({
      id: record.evaluationId,
      userId: record.userId,
      fileName: record.fileName,
      submittedAt: record.submittedAt,
      durationMs: null,
      status: "processing",
    })
  }

  recordFinished(
    evaluationId: string,
    status: Exclude<AudioJobStatus, "processing">,
    finishedAt = Date.now()
  ): void {
    const job = this.readStoredJobs().find(
      (candidate) => candidate.id === evaluationId
    )
    if (job === undefined) return
    this.upsert({
      ...job,
      status,
      durationMs: job.durationMs ?? Math.max(0, finishedAt - job.submittedAt),
    })
  }

  recordResult(evaluationId: string, result: EvaluationResult): void {
    const job = this.readStoredJobs().find(
      (candidate) => candidate.id === evaluationId
    )
    if (job === undefined) return
    this.upsert({
      ...job,
      transcript: result.transcript,
      scores: result.scores,
    })
  }

  getResult(evaluationId: string): EvaluationResult | null {
    const job = this.readStoredJobs().find(
      (candidate) => candidate.id === evaluationId
    )
    if (
      job === undefined ||
      (job.transcript === undefined && job.scores === undefined)
    ) {
      return null
    }
    return {
      transcript: job.transcript,
      scores: job.scores === undefined ? undefined : { ...job.scores },
    }
  }

  private upsert(job: AudioProcessingJob): void {
    const jobs = this.readStoredJobs().filter(
      (candidate) => candidate.id !== job.id
    )
    jobs.push(copyJob(job))
    jobs.sort((left, right) => right.submittedAt - left.submittedAt)
    this.jobs = jobs.slice(0, MAX_JOBS)
    try {
      this.storage?.setItem(
        STORAGE_KEY,
        JSON.stringify({
          version: 1,
          jobs: this.jobs,
        } satisfies StoredEvaluations)
      )
    } catch {
      // The in-memory copy remains usable if quota or privacy settings block writes.
      this.storage = undefined
    }
  }

  private readStoredJobs(): AudioProcessingJob[] {
    if (this.storage === undefined) return this.jobs
    try {
      this.jobs = parseJobs(this.storage.getItem(STORAGE_KEY))
    } catch {
      // Continue with the most recent in-memory snapshot.
      this.storage = undefined
    }
    return this.jobs
  }
}
