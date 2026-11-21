import { describe, expect, it } from "vitest"

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

describe("BrowserEvaluationStore", () => {
  it("persists browser-submitted jobs across store instances", () => {
    const storage = new MemoryStorage()
    const first = new BrowserEvaluationStore(storage)
    first.recordStarted({
      evaluationId: "evaluation-1",
      userId: "user-1",
      fileName: "clip.wav",
      submittedAt: 100,
    })

    expect(new BrowserEvaluationStore(storage).listJobs("user-1")).toEqual([
      {
        id: "evaluation-1",
        userId: "user-1",
        fileName: "clip.wav",
        submittedAt: 100,
        durationMs: null,
        status: "processing",
      },
    ])
  })

  it("stores terminal status and results for previous jobs", () => {
    const storage = new MemoryStorage()
    const store = new BrowserEvaluationStore(storage)
    store.recordStarted({
      evaluationId: "evaluation-1",
      userId: "user-1",
      fileName: "clip.wav",
      submittedAt: 100,
    })
    store.recordFinished("evaluation-1", "complete", 175)
    store.recordResult("evaluation-1", {
      transcript: "hello world",
      scores: {
        sexual: 0.01,
        hate_or_discrimination: 0.02,
        harassment_or_abuse: 0.03,
        violence_or_threats: 0.04,
        asking_for_pii: 0.05,
      },
    })

    const restored = new BrowserEvaluationStore(storage)
    expect(restored.listJobs("user-1")[0]).toMatchObject({
      status: "complete",
      durationMs: 75,
      transcript: "hello world",
    })
    expect(restored.getResult("evaluation-1")?.scores?.asking_for_pii).toBe(
      0.05
    )
  })

  it("isolates users and ignores malformed persisted data", () => {
    const storage = new MemoryStorage()
    storage.setItem("audio-moderation.evaluations.v1", "not json")
    const store = new BrowserEvaluationStore(storage)
    expect(store.listJobs("user-1")).toEqual([])

    store.recordStarted({
      evaluationId: "evaluation-2",
      userId: "user-2",
      fileName: "other.mp3",
      submittedAt: 200,
    })
    expect(store.listJobs("user-1")).toEqual([])
  })

  it("keeps an in-memory history when browser storage rejects writes", () => {
    const store = new BrowserEvaluationStore({
      getItem: () => null,
      setItem: () => {
        throw new Error("quota exceeded")
      },
    })

    store.recordStarted({
      evaluationId: "evaluation-3",
      userId: "user-1",
      fileName: "fallback.wav",
      submittedAt: 300,
    })

    expect(store.listJobs("user-1")).toMatchObject([
      { id: "evaluation-3", status: "processing" },
    ])
  })
})
