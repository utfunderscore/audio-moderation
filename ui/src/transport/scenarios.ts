import { isTerminalEvent } from "@/domain/events"
import type { ModerationScores } from "@/domain/moderation"
import { PIPELINE_STATUS } from "@/domain/reducer"

export interface ScenarioStep {
  event: string
  /** Delay from the previous step, at 1x speed. */
  delayMs: number
}

export interface Scenario {
  id: string
  name: string
  summary: string
  /** Initial PIPELINE_TASK_STATUS_* for the simulated pipeline task. */
  ingressStatus: string
  /** Leading steps delivered as replay when this scenario is chosen. */
  replayPrefix?: number
  steps: ScenarioStep[]
  /**
   * Demo artifacts. The transport only carries event names, so these stand in
   * for the results a real integration would read from the workflow store.
   */
  transcript?: string
  scores?: ModerationScores
}

const ACCEPTED: ScenarioStep = { event: "EVALUATION_ACCEPTED", delayMs: 200 }
const CONVERSION_STARTED: ScenarioStep = {
  event: "AUDIO_PROCESSING_STARTED",
  delayMs: 600,
}
const CONVERSION_FINISHED: ScenarioStep = {
  event: "AUDIO_PROCESSING_FINISHED",
  delayMs: 2200,
}
const ASR_STARTED: ScenarioStep = { event: "ASR_STARTED", delayMs: 400 }
const ASR_FINISHED: ScenarioStep = { event: "ASR_FINISHED", delayMs: 6000 }
const MODERATION_STARTED: ScenarioStep = {
  event: "MODERATION_PROCESSING_STARTED",
  delayMs: 400,
}
const MODERATION_FINISHED: ScenarioStep = {
  event: "MODERATION_PROCESSING_FINISHED",
  delayMs: 4200,
}
const SUCCEEDED: ScenarioStep = { event: "SUCCEEDED", delayMs: 300 }

const DEMO_TRANSCRIPT =
  "Hey, it's Sam from the finance team. Can you send me your home address and " +
  "phone number so I can post the invoice? I'll email the tracking details " +
  "this afternoon."

const DEMO_SCORES: ModerationScores = {
  sexual: 0.02,
  hate_or_discrimination: 0.05,
  harassment_or_abuse: 0.08,
  violence_or_threats: 0.01,
  asking_for_pii: 0.62,
}

const SAFE_TRANSCRIPT =
  "Thanks for the update. The project is on schedule, and I'll share the " +
  "final notes with the team this afternoon."

const SAFE_SCORES: ModerationScores = {
  sexual: 0.01,
  hate_or_discrimination: 0.01,
  harassment_or_abuse: 0.02,
  violence_or_threats: 0.01,
  asking_for_pii: 0.01,
}

const HARASSMENT_TRANSCRIPT =
  "You're completely useless at this job. Everyone knows you always mess " +
  "things up, and nobody wants you on this team."

const HARASSMENT_SCORES: ModerationScores = {
  sexual: 0.01,
  hate_or_discrimination: 0.04,
  harassment_or_abuse: 0.91,
  violence_or_threats: 0.08,
  asking_for_pii: 0.01,
}

const HAPPY_STEPS: ScenarioStep[] = [
  ACCEPTED,
  CONVERSION_STARTED,
  CONVERSION_FINISHED,
  ASR_STARTED,
  ASR_FINISHED,
  MODERATION_STARTED,
  MODERATION_FINISHED,
  SUCCEEDED,
]

export const SCENARIOS: readonly Scenario[] = [
  {
    id: "moderation-safe",
    name: "Safe conversation",
    summary: "Benign content with low scores across all categories",
    ingressStatus: PIPELINE_STATUS.pending,
    steps: HAPPY_STEPS,
    transcript: SAFE_TRANSCRIPT,
    scores: SAFE_SCORES,
  },
  {
    id: "happy",
    name: "PII request",
    summary: "A request for personal information is flagged",
    ingressStatus: PIPELINE_STATUS.pending,
    steps: HAPPY_STEPS,
    transcript: DEMO_TRANSCRIPT,
    scores: DEMO_SCORES,
  },
  {
    id: "moderation-harassment",
    name: "Harassment",
    summary: "Abusive language produces a high harassment score",
    ingressStatus: PIPELINE_STATUS.pending,
    steps: HAPPY_STEPS,
    transcript: HARASSMENT_TRANSCRIPT,
    scores: HARASSMENT_SCORES,
  },
  {
    id: "late-subscribe",
    name: "Late subscribe",
    summary: "Subscribe mid-pipeline, then replay the durable history",
    ingressStatus: PIPELINE_STATUS.startedAsr,
    replayPrefix: 5,
    steps: HAPPY_STEPS,
    transcript: DEMO_TRANSCRIPT,
    scores: DEMO_SCORES,
  },
  {
    id: "duplicates",
    name: "Duplicate frames",
    summary: "At-least-once delivery repeats events at the replay boundary",
    ingressStatus: PIPELINE_STATUS.pending,
    replayPrefix: 3,
    steps: [
      ACCEPTED,
      CONVERSION_STARTED,
      CONVERSION_STARTED,
      CONVERSION_FINISHED,
      ASR_STARTED,
      ASR_STARTED,
      ASR_FINISHED,
      MODERATION_STARTED,
      MODERATION_FINISHED,
      SUCCEEDED,
    ],
    transcript: DEMO_TRANSCRIPT,
    scores: DEMO_SCORES,
  },
  {
    id: "slow-asr",
    name: "Slow ASR",
    summary: "Transcription holds in its partial state for a while",
    ingressStatus: PIPELINE_STATUS.pending,
    steps: [
      ACCEPTED,
      CONVERSION_STARTED,
      CONVERSION_FINISHED,
      ASR_STARTED,
      { event: "ASR_FINISHED", delayMs: 24000 },
      MODERATION_STARTED,
      MODERATION_FINISHED,
      SUCCEEDED,
    ],
    transcript: DEMO_TRANSCRIPT,
    scores: DEMO_SCORES,
  },
  {
    id: "conversion-failure",
    name: "Conversion failure",
    summary: "Stitching fails while audio processing is active",
    ingressStatus: PIPELINE_STATUS.pending,
    steps: [ACCEPTED, CONVERSION_STARTED, { event: "FAILED", delayMs: 2600 }],
  },
  {
    id: "asr-failure",
    name: "Transcription failure",
    summary: "ASR returns a failure callback",
    ingressStatus: PIPELINE_STATUS.pending,
    steps: [
      ACCEPTED,
      CONVERSION_STARTED,
      CONVERSION_FINISHED,
      ASR_STARTED,
      { event: "FAILED", delayMs: 3000 },
    ],
  },
  {
    id: "moderation-failure",
    name: "Moderation failure",
    summary: "Moderation fails after the request was accepted",
    ingressStatus: PIPELINE_STATUS.pending,
    steps: [
      ACCEPTED,
      CONVERSION_STARTED,
      CONVERSION_FINISHED,
      ASR_STARTED,
      ASR_FINISHED,
      MODERATION_STARTED,
      { event: "FAILED", delayMs: 3400 },
    ],
    transcript: DEMO_TRANSCRIPT,
  },
  {
    id: "timeout",
    name: "Moderation timeout",
    summary: "The moderation task token expires",
    ingressStatus: PIPELINE_STATUS.pending,
    steps: [
      ACCEPTED,
      CONVERSION_STARTED,
      CONVERSION_FINISHED,
      ASR_STARTED,
      ASR_FINISHED,
      MODERATION_STARTED,
      { event: "TIMED_OUT", delayMs: 3400 },
    ],
    transcript: DEMO_TRANSCRIPT,
  },
  {
    id: "cancelled",
    name: "Cancelled",
    summary: "The execution is aborted during transcription",
    ingressStatus: PIPELINE_STATUS.pending,
    steps: [
      ACCEPTED,
      CONVERSION_STARTED,
      CONVERSION_FINISHED,
      ASR_STARTED,
      { event: "CANCELLED", delayMs: 2600 },
    ],
  },
  {
    id: "future-event",
    name: "Unknown event",
    summary: "A future backend event is tolerated and ignored",
    ingressStatus: PIPELINE_STATUS.pending,
    steps: [
      ACCEPTED,
      CONVERSION_STARTED,
      CONVERSION_FINISHED,
      ASR_STARTED,
      ASR_FINISHED,
      MODERATION_STARTED,
      { event: "PIPELINE_HEARTBEAT", delayMs: 800 },
      MODERATION_FINISHED,
      SUCCEEDED,
    ],
    transcript: DEMO_TRANSCRIPT,
    scores: DEMO_SCORES,
  },
]

export function getScenario(id: string): Scenario {
  return SCENARIOS.find((scenario) => scenario.id === id) ?? SCENARIOS[0]
}

/** Steps that can be replayed without ending the run. */
export function replayableStepCount(scenario: Scenario): number {
  return scenario.steps.filter((step) => !isTerminalEvent(step.event)).length
}
