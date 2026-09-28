// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react"
import { createElement } from "react"
import { afterEach, describe, expect, it } from "vitest"
import { TranscriptionStage } from "./TranscriptionStage"

afterEach(cleanup)

describe("TranscriptionStage", () => {
  it("does not claim the transcript is missing while it is being fetched", () => {
    render(
      createElement(TranscriptionStage, { state: "complete", loading: true })
    )

    expect(screen.getByText("Loading transcript…")).toBeTruthy()
    expect(screen.queryByText("No transcript returned.")).toBeNull()
  })

  it("shows an empty result when the fetch completes without a transcript", () => {
    render(
      createElement(TranscriptionStage, { state: "complete", loading: false })
    )

    expect(screen.getByText("No transcript returned.")).toBeTruthy()
  })
})
