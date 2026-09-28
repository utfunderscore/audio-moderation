import { useCallback, useEffect, useReducer, useRef, useState } from "react"
import type { Backend } from "@/api/backend"
import type {
  EvaluationResult,
  ResumeEvaluationInput,
  StartEvaluationInput,
  StartedEvaluation,
} from "@/api/evaluation"
import type { PipelineState } from "@/domain/reducer"
import {
  createInitialState,
  isRunning,
  pipelineReducer,
} from "@/domain/reducer"

export interface EvaluationRun {
  state: PipelineState
  /** Transcript after ASR; scores refreshed when the run reaches a terminal event. */
  result: EvaluationResult | null
  start: (input: StartEvaluationInput) => void
  resume: (input: ResumeEvaluationInput) => void
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

  const follow = useCallback(
    (started: StartedEvaluation, token: number, startedAt?: number) => {
      if (runTokenRef.current !== token) return
      dispatch({
        type: "seed",
        evaluationId: started.evaluationId,
        status: started.status,
        at: Date.now(),
        startedAt,
      })
      unsubscribeRef.current = backend.subscribeTaskEvents(
        started.evaluationId,
        {
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
        }
      )
    },
    [backend]
  )

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
        .then((started) => follow(started, token))
        .catch(() => {
          if (runTokenRef.current !== token) return
          dispatch({ type: "connection", state: "error" })
        })
    },
    [backend, follow, teardown]
  )

  const resume = useCallback(
    (input: ResumeEvaluationInput) => {
      runTokenRef.current += 1
      const token = runTokenRef.current
      teardown()
      setResult(null)
      void backend
        .resumeEvaluation(input.evaluationId)
        .then((started) => follow(started, token, input.startedAt))
        .catch(() => {
          if (runTokenRef.current !== token) return
          dispatch({ type: "reset" })
        })
    },
    [backend, follow, teardown]
  )

  useEffect(() => teardown, [teardown])

  // The stream carries names only. Read the transcript as soon as ASR completes,
  // then refresh after the workflow settles to pick up moderation scores.
  useEffect(() => {
    const evaluationId = state.evaluationId
    const outcome = state.outcome
    if (
      evaluationId === null ||
      (state.stages.transcription !== "complete" && outcome === null)
    ) {
      return undefined
    }

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
  }, [backend, state.evaluationId, state.outcome, state.stages.transcription])

  return {
    state,
    result,
    start,
    resume,
    reset,
    running: isRunning(state),
    hasRun: state.startedAt !== null,
  }
}
