import { useCallback, useEffect, useReducer, useRef, useState } from "react"
import type { ConnectionState } from "@/domain/reducer"
import {
  createInitialState,
  isRunning,
  pipelineReducer,
} from "@/domain/reducer"
import type { TaskEventsClient } from "@/transport/client"
import { MockTaskEventsClient } from "@/transport/mockClient"
import { getScenario, SCENARIOS, type Scenario } from "@/transport/scenarios"

export interface PipelineRun {
  state: ReturnType<typeof createInitialState>
  scenarios: readonly Scenario[]
  scenario: Scenario
  scenarioId: string
  selectScenario: (id: string) => void
  speed: number
  setSpeed: (speed: number) => void
  lateSubscribe: boolean
  setLateSubscribe: (value: boolean) => void
  run: (scenarioId?: string) => void
  reset: () => void
  clearEvents: () => void
  running: boolean
  hasRun: boolean
}

export const SPEED_OPTIONS = [1, 4, 16] as const

/**
 * Drives a simulated evaluation: a fake ingress response seeds the tracker,
 * then the mock client replays history and streams live frames.
 */
export function usePipelineRun(
  defaultScenarioId = SCENARIOS[0].id
): PipelineRun {
  const [state, dispatch] = useReducer(
    pipelineReducer,
    undefined,
    createInitialState
  )
  const [scenarioId, setScenarioId] = useState(defaultScenarioId)
  const [speed, setSpeed] = useState<number>(1)
  const [lateSubscribe, setLateSubscribe] = useState(false)

  const clientRef = useRef<TaskEventsClient | null>(null)
  const cleanupRef = useRef<Array<() => void>>([])
  const speedRef = useRef(speed)

  useEffect(() => {
    speedRef.current = speed
  }, [speed])

  const teardown = useCallback(() => {
    for (const cleanup of cleanupRef.current) cleanup()
    cleanupRef.current = []
    clientRef.current?.close()
    clientRef.current = null
  }, [])

  const reset = useCallback(() => {
    teardown()
    dispatch({ type: "reset" })
  }, [teardown])

  const run = useCallback(
    (selectedScenarioId?: string) => {
      teardown()
      dispatch({ type: "reset" })

      const activeScenarioId = selectedScenarioId ?? scenarioId
      const active = getScenario(activeScenarioId)
      if (selectedScenarioId !== undefined) setScenarioId(activeScenarioId)
      const client = new MockTaskEventsClient(
        active,
        () => speedRef.current,
        lateSubscribe
      )
      clientRef.current = client

      cleanupRef.current = [
        client.onFrame((frame) =>
          dispatch({ type: "frame", ...frame, at: Date.now() })
        ),
        client.onConnectionChange((connection: ConnectionState) =>
          dispatch({ type: "connection", state: connection })
        ),
      ]

      const evaluationId = String(1000 + Math.floor(Math.random() * 9000))

      void client
        .connect()
        .then(() => {
          if (clientRef.current !== client) return
          dispatch({
            type: "seed",
            evaluationId,
            status: active.ingressStatus,
            at: Date.now(),
          })
          return client.subscribe(Number(evaluationId))
        })
        .catch(() => {
          // The mock never rejects; keep the promise chain honest for the seam.
        })
    },
    [lateSubscribe, scenarioId, teardown]
  )

  useEffect(() => teardown, [teardown])

  return {
    state,
    scenarios: SCENARIOS,
    scenario: getScenario(scenarioId),
    scenarioId,
    selectScenario: setScenarioId,
    speed,
    setSpeed,
    lateSubscribe,
    setLateSubscribe,
    run,
    reset,
    clearEvents: () => dispatch({ type: "clear-events" }),
    running: isRunning(state),
    hasRun: state.startedAt !== null,
  }
}
