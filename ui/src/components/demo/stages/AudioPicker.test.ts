// @vitest-environment jsdom

import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import { createElement } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { getSampleAudio } from "@/api/sample-audio"
import { TooltipProvider } from "@/components/ui/tooltip"
import type { AudioInputController } from "@/hooks/useAudioInput"
import { AudioPicker } from "./AudioPicker"

vi.mock("@/api/sample-audio", () => ({ getSampleAudio: vi.fn() }))

afterEach(() => {
  cleanup()
  vi.resetAllMocks()
})

function setup(disabled = false) {
  const audio: AudioInputController = {
    file: null,
    url: null,
    audioRef: { current: null },
    selectFile: vi.fn(),
    clear: vi.fn(),
  }
  const onSelected = vi.fn()
  render(
    createElement(
      TooltipProvider,
      null,
      createElement(AudioPicker, { audio, onSelected, disabled })
    )
  )
  return { audio, onSelected }
}

describe("sample buttons", () => {
  it.each([
    ["Neutral", "neutral", "sample_071.mp3"],
    ["Abusive chat", "abusive", "sample-1.mp3"],
  ] as const)(
    "selects %s through the normal file flow",
    async (label, sample, name) => {
      const file = new File(["audio"], name, { type: "audio/mpeg" })
      vi.mocked(getSampleAudio).mockResolvedValue(file)
      const { audio, onSelected } = setup()
      fireEvent.click(screen.getByRole("button", { name: label }))
      expect(getSampleAudio).toHaveBeenCalledWith(
        sample,
        expect.any(AbortSignal)
      )
      await waitFor(() => expect(onSelected).toHaveBeenCalledWith(file))
      expect(audio.selectFile).toHaveBeenCalledWith(file)
    }
  )

  it("reports sample loading errors without selecting a file", async () => {
    vi.mocked(getSampleAudio).mockRejectedValue(new Error("Download failed"))
    const { onSelected } = setup()
    fireEvent.click(screen.getByRole("button", { name: "Neutral" }))
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toBe("Download failed")
    )
    expect(onSelected).not.toHaveBeenCalled()
    expect(
      screen.getByRole("button", { name: "Neutral" }).hasAttribute("disabled")
    ).toBe(false)
  })

  it("does not load samples when selection is disabled", () => {
    setup(true)
    fireEvent.click(screen.getByRole("button", { name: "Neutral" }))
    fireEvent.click(screen.getByRole("button", { name: "Abusive chat" }))
    expect(getSampleAudio).not.toHaveBeenCalled()
  })
})
