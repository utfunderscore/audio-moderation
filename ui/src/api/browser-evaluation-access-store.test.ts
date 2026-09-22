import { describe, expect, it } from "vitest"

import { BrowserEvaluationAccessStore } from "./browser-evaluation-access-store"

class MemoryStorage {
  private readonly values = new Map<string, string>()

  getItem(key: string): string | null {
    return this.values.get(key) ?? null
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value)
  }
}

describe("BrowserEvaluationAccessStore", () => {
  it("restores access after a page reload", () => {
    const storage = new MemoryStorage()
    const first = new BrowserEvaluationAccessStore(storage)
    first.set("evaluation-1", { token: "secret" })

    const restored = new BrowserEvaluationAccessStore(storage)
    expect(restored.get("evaluation-1")).toEqual({ token: "secret" })
  })

  it("removes completed evaluation access", () => {
    const storage = new MemoryStorage()
    const store = new BrowserEvaluationAccessStore(storage)
    store.set("evaluation-1", { token: "secret" })
    store.remove("evaluation-1")

    expect(new BrowserEvaluationAccessStore(storage).get("evaluation-1")).toBe(
      undefined
    )
  })

  it("ignores malformed persisted data", () => {
    const storage = new MemoryStorage()
    storage.setItem("audio-moderation.evaluation-access.v1", "not json")

    expect(new BrowserEvaluationAccessStore(storage).get("evaluation-1")).toBe(
      undefined
    )
  })
})
