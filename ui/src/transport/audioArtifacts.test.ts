import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

describe("demo audio artifact cache", () => {
  beforeEach(() => vi.resetModules())
  afterEach(() => vi.unstubAllGlobals())

  it.each(["network", "http", "body"])(
    "allows a fresh request after a %s failure",
    async (failure) => {
      const blob = new Blob(["audio"], { type: "audio/mpeg" })
      const fetchMock = vi.fn()
      if (failure === "network") {
        fetchMock.mockRejectedValueOnce(new Error("offline"))
      } else {
        fetchMock.mockResolvedValueOnce({
          ok: failure !== "http",
          blob: () => Promise.reject(new Error("interrupted download")),
        })
      }
      fetchMock.mockResolvedValueOnce({ ok: true, blob: async () => blob })
      vi.stubGlobal("fetch", fetchMock)
      const { preloadDemoAudioArtifact } = await import("./audioArtifacts")

      await expect(preloadDemoAudioArtifact()).rejects.toThrow()
      await expect(preloadDemoAudioArtifact()).resolves.toBe(blob)
      await expect(preloadDemoAudioArtifact()).resolves.toBe(blob)
      expect(fetchMock).toHaveBeenCalledTimes(2)
    }
  )

  it("shares the pending download across consumers", async () => {
    const blob = new Blob(["audio"])
    let finish!: (response: Response) => void
    const fetchMock = vi.fn(() => new Promise<Response>((resolve) => {
      finish = resolve
    }))
    vi.stubGlobal("fetch", fetchMock)
    const { preloadDemoAudioArtifact } = await import("./audioArtifacts")

    const first = preloadDemoAudioArtifact()
    const second = preloadDemoAudioArtifact()
    expect(second).toBe(first)
    expect(fetchMock).toHaveBeenCalledTimes(1)
    finish(new Response(blob))
    await expect(first).resolves.toBeInstanceOf(Blob)
  })
})
