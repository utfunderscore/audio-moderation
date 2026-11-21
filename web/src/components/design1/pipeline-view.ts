import type { PipelineState } from "@/domain/reducer";
import { STAGE_IDS, type StageId } from "@/domain/stages";

export type NodeState =
	| "idle"
	| "queued"
	| "processing"
	| "complete"
	| "failed"
	| "skipped";

/** User-facing names for the stages a visitor watches; `result` is the output itself. */
export const STEPS: { id: Exclude<StageId, "result">; label: string }[] = [
	{ id: "submitted", label: "Uploading" },
	{ id: "conversion", label: "Preparing audio" },
	{ id: "transcription", label: "Transcribing" },
	{ id: "moderation", label: "Checking for abuse" },
];

/**
 * The reducer leaves a stage `pending` between its predecessor finishing and
 * its own start event; treat that gap as `queued` so progress never stalls.
 */
export function nodeStates(
	state: PipelineState,
	submitting: boolean,
): Record<StageId, NodeState> {
	const live =
		(submitting || state.startedAt !== null) && state.outcome === null;
	const result = {} as Record<StageId, NodeState>;
	let previousDone = true;
	for (const id of STAGE_IDS) {
		const raw = state.stages[id];
		if (raw === "pending") {
			if (live && previousDone) {
				result[id] = id === "submitted" ? "processing" : "queued";
			} else {
				result[id] = "idle";
			}
		} else {
			result[id] = raw;
		}
		previousDone = raw === "complete";
	}
	return result;
}
