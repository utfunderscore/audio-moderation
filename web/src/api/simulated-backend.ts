import type { AudioProcessingJob } from "@/domain/jobs";
import type { ModerationScores } from "@/domain/moderation";
import { PIPELINE_STATUS } from "@/domain/reducer";
import { SAMPLE_JOB } from "@/domain/sample-job";
import type { Backend, TaskEventHandlers, Unsubscribe } from "./backend";
import type {
	EvaluationResult,
	StartEvaluationInput,
	StartedEvaluation,
} from "./evaluation";

interface Fixture {
	transcript: string;
	scores: ModerationScores;
}

// Design-only fixtures. The neutral transcript is placeholder copy, not a
// transcription of sample_071.mp3.
const FIXTURES: Record<string, Fixture> = {
	[SAMPLE_JOB.fileName]: {
		transcript: SAMPLE_JOB.transcript ?? "",
		scores: SAMPLE_JOB.scores ?? emptyScores(),
	},
	"sample_071.mp3": {
		transcript:
			"Okay, rotate to B, I've got the smoke. Nice shot. Two left, one of them is low. Let's regroup and full-buy next round.",
		scores: {
			sexual: 0.01,
			hate_or_discrimination: 0.02,
			harassment_or_abuse: 0.06,
			violence_or_threats: 0.03,
			asking_for_pii: 0.01,
		},
	},
};

function emptyScores(): ModerationScores {
	return {
		sexual: 0,
		hate_or_discrimination: 0,
		harassment_or_abuse: 0,
		violence_or_threats: 0,
		asking_for_pii: 0,
	};
}

/**
 * Event script replayed over the socket, as offsets from subscription. One
 * frame is repeated to exercise the at-least-once delivery the reducer absorbs.
 */
const SCRIPT: readonly [delayMs: number, event: string][] = [
	[350, "EVALUATION_ACCEPTED"],
	[900, "AUDIO_PROCESSING_STARTED"],
	[2300, "AUDIO_PROCESSING_FINISHED"],
	[2700, "ASR_STARTED"],
	[3100, "ASR_STARTED"],
	[5600, "ASR_FINISHED"],
	[6000, "MODERATION_PROCESSING_STARTED"],
	[8200, "MODERATION_PROCESSING_FINISHED"],
	[8600, "SUCCEEDED"],
];

interface Run {
	fixture: Fixture;
	emitted: Set<string>;
}

/**
 * In-browser stand-in for `ApiBackend` that drives the real hook and reducer
 * with the production event names, without Turnstile, uploads or AWS.
 */
export class SimulatedBackend implements Backend {
	private readonly runs = new Map<string, Run>();
	private sequence = 0;

	async listJobs(_userId: string): Promise<AudioProcessingJob[]> {
		return [];
	}

	async startEvaluation(
		input: StartEvaluationInput,
	): Promise<StartedEvaluation> {
		await delay(450);
		this.sequence += 1;
		const evaluationId = `ev_${randomHex(4)}${this.sequence.toString(16).padStart(2, "0")}`;
		this.runs.set(evaluationId, {
			fixture: FIXTURES[input.audio.name] ?? FIXTURES["sample_071.mp3"],
			emitted: new Set(),
		});
		return { evaluationId, status: PIPELINE_STATUS.pending };
	}

	async resumeEvaluation(evaluationId: string): Promise<StartedEvaluation> {
		if (!this.runs.has(evaluationId)) throw new Error("Unknown evaluation");
		return { evaluationId, status: PIPELINE_STATUS.pending };
	}

	async getEvaluationResult(
		evaluationId: string,
	): Promise<EvaluationResult | null> {
		const run = this.runs.get(evaluationId);
		if (run === undefined) return null;
		await delay(250);
		return {
			transcript: run.emitted.has("ASR_FINISHED")
				? run.fixture.transcript
				: undefined,
			scores: run.emitted.has("MODERATION_PROCESSING_FINISHED")
				? run.fixture.scores
				: undefined,
		};
	}

	async getJobAudio(_jobId: string): Promise<Blob> {
		throw new Error("Job audio is not available in the simulated backend");
	}

	subscribeTaskEvents(
		evaluationId: string,
		handlers: TaskEventHandlers,
	): Unsubscribe {
		const run = this.runs.get(evaluationId);
		const timers: ReturnType<typeof setTimeout>[] = [];
		handlers.onConnectionChange?.("connecting");
		timers.push(
			setTimeout(() => handlers.onConnectionChange?.("subscribed"), 200),
		);
		for (const [at, name] of SCRIPT) {
			timers.push(
				setTimeout(() => {
					run?.emitted.add(name);
					handlers.onFrame({ name });
				}, at),
			);
		}
		const last = SCRIPT[SCRIPT.length - 1][0];
		timers.push(
			setTimeout(() => handlers.onConnectionChange?.("closed"), last + 300),
		);
		return () => {
			for (const timer of timers) clearTimeout(timer);
		};
	}
}

function delay(ms: number) {
	return new Promise((resolve) => setTimeout(resolve, ms));
}

function randomHex(bytes: number) {
	return Array.from(crypto.getRandomValues(new Uint8Array(bytes)), (b) =>
		b.toString(16).padStart(2, "0"),
	).join("");
}
