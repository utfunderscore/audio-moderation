import { type AudioProcessingJob, DEMO_USER_ID } from "@/domain/jobs"

/** UI-only example. Never persist or send this ID to the backend. */
export const SAMPLE_JOB: AudioProcessingJob = {
  id: "sample-job",
  userId: DEMO_USER_ID,
  fileName: "sample-1.mp3",
  submittedAt: 0,
  durationMs: 21_014,
  status: "complete",
  transcript:
    "Fucking watching. I asked you to watch my body. You heard the TPU idiots. Goddamn you're so fucking bad, Stan. You're so fucking bad. You didn't fucking stay back to watch it. What are you watching? I'm using the Alt. What are you watching? Why would you try to get the orb? What I'm saying, they're fucking there from the beginning.",
  scores: {
    sexual: 0.01,
    hate_or_discrimination: 0.01,
    harassment_or_abuse: 0.82,
    violence_or_threats: 0,
    asking_for_pii: 0,
  },
}

export function isSampleJob(job: AudioProcessingJob): boolean {
  return job.id === SAMPLE_JOB.id
}
