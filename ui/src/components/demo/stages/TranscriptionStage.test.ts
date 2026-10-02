// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react"
import { createElement } from "react"
import { afterEach, expect, it } from "vitest"
import { TranscriptionStage } from "./TranscriptionStage"

afterEach(cleanup)

it("shows a loading message while the completed transcript is being fetched", () => {
  render(
    createElement(TranscriptionStage, { state: "complete", loading: true })
  )
  expect(screen.getByRole("status").textContent).toBe("Loading transcript…")
  expect(screen.queryByText("No transcript returned.")).toBeNull()
})

it("keeps an available transcript visible while refreshing results", () => {
  render(
    createElement(TranscriptionStage, {
      state: "complete",
      loading: true,
      transcript: "Hello world.",
    })
  )
  expect(screen.getByText("Hello world.")).toBeDefined()
  expect(screen.queryByRole("status")).toBeNull()
})
