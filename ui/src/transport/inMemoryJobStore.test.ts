import { describe, expect, it } from "vitest"

import type { AudioProcessingJob } from "@/domain/jobs"
import { InMemoryAudioJobStore } from "./inMemoryJobStore"

const JOBS: readonly AudioProcessingJob[] = [
  {
    id: "older",
    userId: "user-a",
    fileName: "older.mp3",
    submittedAt: 100,
    durationMs: 10,
    status: "complete",
  },
  {
    id: "newer",
    userId: "user-a",
    fileName: "newer.mp3",
    submittedAt: 200,
    durationMs: null,
    status: "processing",
  },
  {
    id: "other-user",
    userId: "user-b",
    fileName: "private.mp3",
    submittedAt: 300,
    durationMs: 20,
    status: "complete",
  },
]

describe("InMemoryAudioJobStore", () => {
  it("fetches only the current user's jobs in newest-first order", async () => {
    const store = new InMemoryAudioJobStore(JOBS)

    const jobs = await store.listJobs("user-a")

    expect(jobs.map((job) => job.id)).toEqual(["newer", "older"])
  })

  it("upserts completed jobs", async () => {
    const store = new InMemoryAudioJobStore([])
    const job = JOBS[0]

    await store.upsertJob(job)

    expect(await store.listJobs("user-a")).toEqual([job])
  })

  it("returns copies rather than exposing stored records", async () => {
    const store = new InMemoryAudioJobStore(JOBS)
    const jobs = await store.listJobs("user-a")
    jobs[0].fileName = "changed.mp3"

    expect((await store.listJobs("user-a"))[0].fileName).toBe("newer.mp3")
  })
})
