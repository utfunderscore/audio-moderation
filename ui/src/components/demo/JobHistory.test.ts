// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { createElement } from "react"
import { afterEach, beforeEach, expect, it, vi } from "vitest"
import type { AudioProcessingJob } from "@/domain/jobs"
import { JobHistory } from "./JobHistory"

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

beforeEach(() => {
  vi.stubGlobal("matchMedia", () => ({
    matches: true,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }))
})

const jobs: AudioProcessingJob[] = Array.from({ length: 21 }, (_, index) => ({
  id: String(index + 1),
  userId: "demo-user",
  fileName: `audio-${index + 1}.mp3`,
  submittedAt: 1_000,
  durationMs: 1_000,
  status: "complete",
}))

const props = {
  currentJob: null,
  jobs,
  loading: false,
  error: null,
  selectedJobId: null,
  onOpenCurrent: vi.fn(),
  onSelectJob: vi.fn(),
}

it("limits desktop pages to ten jobs and disables boundary controls", () => {
  render(createElement(JobHistory, props))
  expect(screen.getAllByRole("row")).toHaveLength(11)
  expect(
    screen.getByRole("button", { name: "Previous" }).hasAttribute("disabled")
  ).toBe(true)
  fireEvent.click(screen.getByRole("button", { name: "Next" }))
  expect(screen.getByText("Showing 11–20 of 21 jobs")).toBeDefined()
  expect(screen.getAllByRole("row")).toHaveLength(11)
  fireEvent.click(screen.getByRole("button", { name: "Next" }))
  expect(screen.getAllByRole("row")).toHaveLength(2)
  expect(
    screen.getByRole("button", { name: "Next" }).hasAttribute("disabled")
  ).toBe(true)
  fireEvent.click(screen.getByRole("button", { name: "Previous" }))
  fireEvent.click(
    screen.getByRole("row", { name: "Open job 11: audio-11.mp3" })
  )
  expect(props.onSelectJob).toHaveBeenCalledWith(jobs[10])
})

it("also paginates the mobile list", () => {
  vi.stubGlobal("matchMedia", () => ({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }))
  render(createElement(JobHistory, props))
  expect(screen.getAllByRole("button", { name: /^Open job/ })).toHaveLength(10)
  fireEvent.click(screen.getByRole("button", { name: "Next" }))
  expect(
    screen.getByRole("button", { name: "Open job 11: audio-11.mp3" })
  ).toBeDefined()
})

it("counts the current job once within the page limit", () => {
  render(
    createElement(JobHistory, {
      ...props,
      currentJob: {
        id: jobs[0].id,
        fileName: jobs[0].fileName,
        startedAt: 1_000,
        endedAt: 2_000,
        status: "complete",
      },
    })
  )
  expect(screen.getAllByRole("row")).toHaveLength(11)
  expect(screen.getAllByText("audio-1.mp3")).toHaveLength(1)
  expect(screen.getByText("Showing 1–10 of 21 jobs")).toBeDefined()
})

it("clamps the page when the list shrinks and hides controls for an empty list", () => {
  const { rerender } = render(createElement(JobHistory, props))
  fireEvent.click(screen.getByRole("button", { name: "Next" }))
  fireEvent.click(screen.getByRole("button", { name: "Next" }))
  rerender(createElement(JobHistory, { ...props, jobs: jobs.slice(0, 11) }))
  expect(screen.getByText("Page 2 of 2")).toBeDefined()
  expect(screen.getByText("audio-11.mp3")).toBeDefined()
  rerender(createElement(JobHistory, { ...props, jobs: [] }))
  expect(screen.getByText("No recent jobs.")).toBeDefined()
  expect(screen.queryByRole("navigation")).toBeNull()
})
