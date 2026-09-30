import { afterEach, describe, expect, it, vi } from "vitest"
import { getSampleAudio } from "./sample-audio"

afterEach(() => vi.unstubAllGlobals())

describe("getSampleAudio", () => {
  it.each([
    ["neutral", "sample_071.mp3"],
    ["abusive", "sample-1.mp3"],
  ] as const)(
    "loads the %s sample as an uploadable MP3",
    async (sample, name) => {
      const fetchMock = vi
        .fn()
        .mockResolvedValue(
          new Response(new Blob(["audio"], { type: "audio/mpeg" }))
        )
      vi.stubGlobal("fetch", fetchMock)
      const signal = new AbortController().signal
      const file = await getSampleAudio(sample, signal)
      expect(fetchMock).toHaveBeenCalledWith(`/samples/${name}`, { signal })
      expect(file.name).toBe(name)
      expect(file.type).toBe("audio/mpeg")
      expect(await file.text()).toBe("audio")
    }
  )

  it("reports failed downloads", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(new Response(null, { status: 404 }))
    )
    await expect(
      getSampleAudio("neutral", new AbortController().signal)
    ).rejects.toThrow("Unable to load sample audio")
  })

  it("rejects an HTML fallback instead of uploading it", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response("<html></html>", {
          headers: { "Content-Type": "text/html" },
        })
      )
    )
    await expect(
      getSampleAudio("neutral", new AbortController().signal)
    ).rejects.toThrow("Sample audio is unavailable")
  })
})
