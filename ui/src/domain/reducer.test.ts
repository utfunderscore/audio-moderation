import { describe, expect, it } from "vitest"

import {
  createInitialState,
  PIPELINE_STATUS,
  type PipelineState,
  pipelineReducer,
} from "./reducer"

function feed(
  state: PipelineState,
  names: string[],
  startAt = 1_000,
  source: "replay" | "live" = "live"
): PipelineState {
  return names.reduce(
    (current, name, index) =>
      pipelineReducer(current, {
        type: "frame",
        name,
        source,
        at: startAt + index,
      }),
    state
  )
}

const HAPPY_PATH = [
  "EVALUATION_ACCEPTED",
  "AUDIO_PROCESSING_STARTED",
  "AUDIO_PROCESSING_FINISHED",
  "ASR_STARTED",
  "ASR_FINISHED",
  "MODERATION_PROCESSING_STARTED",
  "MODERATION_PROCESSING_FINISHED",
  "SUCCEEDED",
]

describe("pipelineReducer", () => {
  it("advances every stage through the happy path", () => {
    const state = feed(createInitialState(), HAPPY_PATH)

    expect(state.outcome).toBe("SUCCEEDED")
    expect(state.stages).toEqual({
      submitted: "complete",
      conversion: "complete",
      transcription: "complete",
      moderation: "complete",
      result: "complete",
    })
    expect(state.events).toHaveLength(HAPPY_PATH.length)
  })

  it("exposes partial stage changes while a stage is running", () => {
    const state = feed(createInitialState(), [
      "EVALUATION_ACCEPTED",
      "AUDIO_PROCESSING_STARTED",
      "AUDIO_PROCESSING_FINISHED",
      "ASR_STARTED",
    ])

    expect(state.stages.conversion).toBe("complete")
    expect(state.stages.transcription).toBe("processing")
    expect(state.stages.moderation).toBe("pending")
    expect(state.stageTimes.transcription.startedAt).toBeDefined()
    expect(state.stageTimes.transcription.endedAt).toBeUndefined()
  })

  it("flags duplicates without changing state", () => {
    let state = feed(createInitialState(), [
      "EVALUATION_ACCEPTED",
      "AUDIO_PROCESSING_STARTED",
    ])
    state = pipelineReducer(state, {
      type: "frame",
      name: "AUDIO_PROCESSING_STARTED",
      source: "replay",
      at: 5_000,
    })

    expect(state.events).toHaveLength(3)
    expect(state.events[2].duplicate).toBe(true)
    expect(state.stages.conversion).toBe("processing")
  })

  it("never regresses a completed stage when replay arrives out of order", () => {
    let state = pipelineReducer(createInitialState(), {
      type: "seed",
      evaluationId: "42",
      status: PIPELINE_STATUS.startedAsr,
      at: 1_000,
    })
    state = feed(
      state,
      [
        "EVALUATION_ACCEPTED",
        "AUDIO_PROCESSING_STARTED",
        "AUDIO_PROCESSING_FINISHED",
      ],
      1_100,
      "replay"
    )

    expect(state.stages.submitted).toBe("complete")
    expect(state.stages.conversion).toBe("complete")
    expect(state.stages.transcription).toBe("processing")
  })

  it("logs unknown events without touching the stages", () => {
    const state = feed(createInitialState(), ["PIPELINE_HEARTBEAT"])

    expect(state.events[0].name).toBe("PIPELINE_HEARTBEAT")
    expect(state.stages).toEqual(createInitialState().stages)
  })

  it("fails the active stage and skips the rest on a failure", () => {
    const state = feed(createInitialState(), [
      "EVALUATION_ACCEPTED",
      "AUDIO_PROCESSING_STARTED",
      "FAILED",
    ])

    expect(state.outcome).toBe("FAILED")
    expect(state.stages.conversion).toBe("failed")
    expect(state.stages.transcription).toBe("skipped")
    expect(state.stages.moderation).toBe("skipped")
    expect(state.stages.result).toBe("failed")
  })

  it("keeps a terminal outcome even if later frames arrive", () => {
    let state = feed(createInitialState(), ["EVALUATION_ACCEPTED", "FAILED"])
    state = pipelineReducer(state, {
      type: "frame",
      name: "SUCCEEDED",
      source: "live",
      at: 9_999,
    })

    expect(state.outcome).toBe("FAILED")
    expect(state.events).toHaveLength(3)
  })

  it("seeds a partial state from the ingress status", () => {
    const state = pipelineReducer(createInitialState(), {
      type: "seed",
      evaluationId: "1042",
      status: PIPELINE_STATUS.asrFinished,
      at: 5_000,
    })

    expect(state.evaluationId).toBe("1042")
    expect(state.startedAt).toBe(5_000)
    expect(state.stages).toMatchObject({
      submitted: "complete",
      conversion: "complete",
      transcription: "complete",
      moderation: "pending",
    })
  })

  it("clears the event log without resetting stages", () => {
    let state = feed(createInitialState(), ["EVALUATION_ACCEPTED"])
    state = pipelineReducer(state, { type: "clear-events" })

    expect(state.events).toHaveLength(0)
    expect(state.stages.submitted).toBe("complete")
  })

  it("resets everything back to the initial state", () => {
    const running = feed(createInitialState(), HAPPY_PATH)
    const state = pipelineReducer(running, { type: "reset" })

    expect(state).toEqual(createInitialState())
  })
})
