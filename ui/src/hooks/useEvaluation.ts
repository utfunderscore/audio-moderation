import { useCallback, useEffect, useReducer, useRef, useState } from "react"
import type {
  Backend,
  EvaluationResult,
  StartEvaluationInput,
} from "@/api/backend"
import type { PipelineState } from "@/domain/reducer"
import {
  createInitialState,
  isRunning,
  pipelineReducer,
} from "@/domain/reducer"

export interface EvaluationRun {
  state: PipelineState
  /** Transcript and scores fetched once the run reaches a terminal event. */
  result: EvaluationResult | null
  start: (input: StartEvaluationInput) => void
  reset: () => void
  running: boolean
  hasRun: boolean
}

/**
 * Drives one evaluation: upload the audio, seed the stage tracker from the
 * ingress status, then follow the evaluation's task-event stream. All backend
 * access goes through the injected `Backend`.
 */
export function useEvaluation(backend: Backend): EvaluationRun {
  const [state, dispatch] = useReducer(
    pipelineReducer,
    undefined,
    createInitialState
  )
  const [result, setResult] = useState<EvaluationResult | null>(null)
  const unsubscribeRef = useRef<(() => void) | null>(null)
  const runTokenRef = useRef(0)

  const teardown = useCallback(() => {
    unsubscribeRef.current?.()
    unsubscribeRef.current = null
  }, [])

  const reset = useCallback(() => {
    runTokenRef.current += 1
    teardown()
    setResult(null)
    dispatch({ type: "reset" })
  }, [teardown])

  const start = useCallback(
    (input: StartEvaluationInput) => {
      runTokenRef.current += 1
      const token = runTokenRef.current
      teardown()
      setResult(null)
      dispatch({ type: "reset" })
      dispatch({ type: "connection", state: "connecting" })

      void backend
        .startEvaluation(input)
        .then(({ evaluationId, status }) => {
          if (runTokenRef.current !== token) return
          dispatch({
            type: "seed",
            evaluationId,
            status,
            at: Date.now(),
          })
          unsubscribeRef.current = backend.subscribeTaskEvents(evaluationId, {
            onFrame: (frame) =>
              dispatch({
                type: "frame",
                name: frame.name,
                source: "live",
                at: Date.now(),
              }),
            onConnectionChange: (connection) =>
              dispatch({ type: "connection", state: connection }),
            onError: () => dispatch({ type: "connection", state: "error" }),
          })
        })
        .catch(() => {
          if (runTokenRef.current !== token) return
          dispatch({ type: "connection", state: "error" })
        })
    },
    [backend, teardown]
  )

  useEffect(() => teardown, [teardown])

  // The event stream carries names only; read the persisted artifacts once the
  // workflow settles.
  useEffect(() => {
    const evaluationId = state.evaluationId
    const outcome = state.outcome
    if (outcome === null || evaluationId === null) return undefined

    let cancelled = false
    void backend
      .getEvaluationResult(evaluationId)
      .then((next) => {
        if (cancelled) return
        setResult(next)
      })
      .catch(() => {
        // A missing result is rendered as "No scores/transcript returned".
      })

    return () => {
      cancelled = true
    }
  }, [backend, state.evaluationId, state.outcome])

  return {
    state,
    result,
    start,
    reset,
    running: isRunning(state),
    hasRun: state.startedAt !== null,
  }
}
