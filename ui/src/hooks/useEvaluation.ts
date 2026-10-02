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
  /** Persisted artifacts fetched as each stage completes and at termination. */
  result: EvaluationResult | null
  resultLoading: boolean
  submitting: boolean
  submissionError: string | null
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
  const [resultLoading, setResultLoading] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [submissionError, setSubmissionError] = useState<string | null>(null)
  const submittingRef = useRef(false)
  const unsubscribeRef = useRef<(() => void) | null>(null)
  const runTokenRef = useRef(0)

  const teardown = useCallback(() => {
    unsubscribeRef.current?.()
    unsubscribeRef.current = null
  }, [])

  const reset = useCallback(() => {
    runTokenRef.current += 1
    submittingRef.current = false
    setSubmitting(false)
    setSubmissionError(null)
    teardown()
    setResult(null)
    setResultLoading(false)
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
        startedAt: started.startedAt ?? startedAt,
        stageTimes: started.stageTimes,
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
      if (submittingRef.current) return
      submittingRef.current = true
      setSubmitting(true)
      setSubmissionError(null)
      const startedAt = Date.now()
      runTokenRef.current += 1
      const token = runTokenRef.current
      teardown()
      setResult(null)
      setResultLoading(false)
      dispatch({ type: "reset" })
      dispatch({ type: "connection", state: "connecting" })

      void backend
        .startEvaluation(input)
        .then((started) => {
          if (runTokenRef.current !== token) return
          submittingRef.current = false
          setSubmitting(false)
          follow(started, token, startedAt)
        })
        .catch((error: unknown) => {
          if (runTokenRef.current !== token) return
          submittingRef.current = false
          setSubmitting(false)
          setSubmissionError(
            error instanceof Error ? error.message : "Unable to submit audio"
          )
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
      setResultLoading(false)
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

  const transcriptionComplete = state.stages.transcription === "complete"
  const moderationComplete = state.stages.moderation === "complete"

  // The event stream carries names only. Read each artifact as soon as its
  // stage completes, including when resuming an in-progress evaluation.
  useEffect(() => {
    const evaluationId = state.evaluationId
    const outcome = state.outcome
    if (
      evaluationId === null ||
      (!transcriptionComplete && !moderationComplete && outcome === null)
    )
      return undefined

    let cancelled = false
    setResultLoading(true)
    void backend
      .getEvaluationResult(evaluationId)
      .then((next) => {
        if (cancelled) return
        if (next !== null) setResult(next)
      })
      .catch(() => {
        // A missing result is rendered as "No scores/transcript returned".
      })
      .finally(() => {
        if (!cancelled) setResultLoading(false)
      })

    return () => {
      cancelled = true
    }
  }, [
    backend,
    state.evaluationId,
    state.outcome,
    transcriptionComplete,
    moderationComplete,
  ])

  return {
    state,
    result,
    resultLoading,
    submitting,
    submissionError,
    start,
    resume,
    reset,
    running: isRunning(state),
    hasRun: state.startedAt !== null,
  }
}
