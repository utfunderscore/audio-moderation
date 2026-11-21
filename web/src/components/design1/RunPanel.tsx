import { Button } from "@ui-sdk/react/button";
import {
	Meter,
	MeterIndicator,
	MeterLabel,
	MeterTrack,
	MeterValue,
} from "@ui-sdk/react/meter";

import type { EvaluationResult } from "@/api/evaluation";
import {
	flaggedCategories,
	MODERATION_CATEGORIES,
	MODERATION_FLAG_THRESHOLD,
	type ModerationScores,
} from "@/domain/moderation";
import type { PipelineState } from "@/domain/reducer";
import type { StageId } from "@/domain/stages";

import { AudioPlayer } from "./AudioPlayer";
import { ArrowRightIcon, RestartIcon } from "./icons";
import type { NodeState } from "./pipeline-view";
import { StageTrack } from "./StageTrack";

type RunPanelProps = {
	state: PipelineState;
	nodes: Record<StageId, NodeState>;
	result: EvaluationResult | null;
	file: File | null;
	onReset: () => void;
};

export function RunPanel({
	state,
	nodes,
	result,
	file,
	onReset,
}: RunPanelProps) {
	const scores = result?.scores;
	const transcript = result?.transcript;
	const done = state.outcome !== null;
	const failed = done && state.outcome !== "SUCCEEDED";

	return (
		<div className="d1-run">
			<StageTrack nodes={nodes} collapsed={done && !failed} />

			{failed ? (
				<div className="d1-verdict" data-tone="failure">
					<h2>We couldn't check this clip</h2>
					<p>Something went wrong while processing it. Try another file.</p>
				</div>
			) : scores ? (
				<Verdict scores={scores} />
			) : (
				<div className="d1-verdict" data-tone="pending">
					<h2>Checking your clip…</h2>
					<p>This usually takes a few seconds.</p>
				</div>
			)}

			{file && <AudioPlayer file={file} />}

			{!failed && (
				<div className="d1-scores" data-ready={scores ? "" : undefined}>
					{MODERATION_CATEGORIES.map((category) => {
						const value = scores ? Math.round(scores[category.key] * 100) : 0;
						const flagged =
							!!scores && scores[category.key] >= MODERATION_FLAG_THRESHOLD;
						return (
							<Meter
								key={category.key}
								className="d1-meter"
								value={value}
								data-flagged={flagged ? "" : undefined}
							>
								<MeterLabel>{category.label}</MeterLabel>
								{scores ? (
									<MeterValue />
								) : (
									<span className="d1-meter-empty" aria-hidden="true" />
								)}
								<MeterTrack>
									<MeterIndicator />
								</MeterTrack>
							</Meter>
						);
					})}
				</div>
			)}

			{!failed && (
				<section
					className="d1-transcript"
					aria-labelledby="d1-transcript-title"
				>
					<h3 id="d1-transcript-title">What was said</h3>
					{transcript ? (
						<p>{transcript}</p>
					) : (
						<div className="d1-skeleton" aria-hidden="true">
							<span />
							<span />
							<span />
						</div>
					)}
				</section>
			)}

			{done && (
				<div className="d1-next">
					<Button
						className="d1-button"
						data-variant="primary"
						type="button"
						onClick={onReset}
					>
						<RestartIcon />
						Check another clip
					</Button>
					<Button className="d1-button" data-variant="ghost" type="button">
						Use it in your app
						<ArrowRightIcon />
					</Button>
				</div>
			)}
		</div>
	);
}

function Verdict({ scores }: { scores: ModerationScores }) {
	const flagged = flaggedCategories(scores);
	if (flagged.length === 0) {
		return (
			<div className="d1-verdict" data-tone="clean">
				<h2>Looks clean</h2>
				<p>
					Nothing in this clip scored above{" "}
					{Math.round(MODERATION_FLAG_THRESHOLD * 100)}%.
				</p>
			</div>
		);
	}
	const top = flagged.reduce((a, b) => (scores[b.key] > scores[a.key] ? b : a));
	return (
		<div className="d1-verdict" data-tone="flagged">
			<h2>Flagged</h2>
			<p>
				{flagged.map((c) => c.label).join(", ")}, scoring{" "}
				{Math.round(scores[top.key] * 100)}%.
			</p>
		</div>
	);
}
