import type { AudioJobStore, AudioProcessingJob } from "@/domain/jobs"
import { DEMO_USER_ID } from "@/domain/jobs"

const minute = 60_000
const day = 24 * 60 * minute

const SAFE_TRANSCRIPT =
  "We can meet in the game lobby after lunch and review the new level together."
const REVIEW_TRANSCRIPT =
  "Send me your home address and phone number so I can mail the prize to you."
const SAFE_SCORES = {
  sexual: 0.01,
  hate_or_discrimination: 0.02,
  harassment_or_abuse: 0.04,
  violence_or_threats: 0.01,
  asking_for_pii: 0.08,
} as const

function submittedAt(daysAgo: number, hour: number, minuteValue: number) {
  const date = new Date(Date.now() - daysAgo * day)
  date.setHours(hour, minuteValue, 0, 0)
  return date.getTime()
}

const INITIAL_JOBS: readonly AudioProcessingJob[] = [
  {
    id: "9471",
    userId: DEMO_USER_ID,
    fileName: "voice-chat-09-17.mp3",
    submittedAt: submittedAt(0, 9, 42),
    durationMs: 2 * minute + 18_000,
    status: "complete",
    transcript: SAFE_TRANSCRIPT,
    scores: SAFE_SCORES,
  },
  {
    id: "9318",
    userId: DEMO_USER_ID,
    fileName: "lobby-session.wav",
    submittedAt: submittedAt(1, 16, 16),
    durationMs: 46_000,
    status: "complete",
    transcript: REVIEW_TRANSCRIPT,
    scores: {
      ...SAFE_SCORES,
      asking_for_pii: 0.74,
    },
  },
  {
    id: "9104",
    userId: DEMO_USER_ID,
    fileName: "party-audio-07.mp3",
    submittedAt: submittedAt(2, 11, 8),
    durationMs: 5 * minute + 3_000,
    status: "failed",
  },
  {
    id: "8892",
    userId: DEMO_USER_ID,
    fileName: "gameplay-clip.m4a",
    submittedAt: submittedAt(5, 14, 31),
    durationMs: minute + 27_000,
    status: "complete",
    transcript: SAFE_TRANSCRIPT,
    scores: SAFE_SCORES,
  },
]

function keyFor(job: Pick<AudioProcessingJob, "userId" | "id">) {
  return `${job.userId}:${job.id}`
}

/**
 * Temporary module-local implementation of the jobs API seam. Data survives
 * component remounts, but intentionally resets when the browser page reloads.
 */
export class InMemoryAudioJobStore implements AudioJobStore {
  private readonly jobs = new Map<string, AudioProcessingJob>()

  constructor(initialJobs: readonly AudioProcessingJob[] = INITIAL_JOBS) {
    for (const job of initialJobs) this.jobs.set(keyFor(job), { ...job })
  }

  async listJobs(userId: string): Promise<AudioProcessingJob[]> {
    await Promise.resolve()
    return [...this.jobs.values()]
      .filter((job) => job.userId === userId)
      .sort((left, right) => right.submittedAt - left.submittedAt)
      .map((job) => ({ ...job }))
  }

  async upsertJob(job: AudioProcessingJob): Promise<void> {
    await Promise.resolve()
    this.jobs.set(keyFor(job), { ...job })
  }
}

export const inMemoryAudioJobStore = new InMemoryAudioJobStore()
