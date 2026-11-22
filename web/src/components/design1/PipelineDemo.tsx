import {
	type ReactNode,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";

import type { AudioSample } from "@/api/sample-audio";
import type { EvaluationRun } from "@/hooks/useEvaluation";

import { ClipInput } from "./ClipInput";
import { nodeStates, STEPS } from "./pipeline-view";
import { RunPanel } from "./RunPanel";
import { VerifyPanel } from "./VerifyPanel";

const useIsoLayoutEffect =
	typeof window === "undefined" ? useEffect : useLayoutEffect;

/** Animates the card's height as its content swaps between upload and result. */
function AutoHeight({ children }: { children: ReactNode }) {
	const inner = useRef<HTMLDivElement>(null);
	const [height, setHeight] = useState<number | null>(null);
	const [animating, setAnimating] = useState(false);

	useIsoLayoutEffect(() => {
		const el = inner.current;
		if (!el) return;
		const observer = new ResizeObserver(() => {
			setHeight((prev) => {
				const next = el.offsetHeight;
				if (prev !== null && prev !== next) setAnimating(true);
				return next;
			});
		});
		observer.observe(el);
		return () => observer.disconnect();
	}, []);

	return (
		<div
			className="d1-autoheight"
			data-animating={animating ? "" : undefined}
			style={height === null ? undefined : { height }}
			onTransitionEnd={(e) => {
				if (e.target === e.currentTarget && e.propertyName === "height") {
					setAnimating(false);
				}
			}}
		>
			<div ref={inner}>{children}</div>
		</div>
	);
}

type PipelineDemoProps = {
	evaluation: EvaluationRun;
	file: File | null;
	verifying: File | null;
	turnstileSiteKey: string;
	error: string | null;
	onFile: (file: File) => void;
	onVerified: (token: string) => void;
	onReset: () => void;
	loadSample: (sample: AudioSample, signal: AbortSignal) => Promise<File>;
};

export function PipelineDemo({
	evaluation,
	file,
	verifying,
	turnstileSiteKey,
	error,
	onFile,
	onVerified,
	onReset,
	loadSample,
}: PipelineDemoProps) {
	const { state, submitting, result } = evaluation;
	const started = submitting || evaluation.hasRun;
	const nodes = nodeStates(state, submitting);

	return (
		<section className="d1-card" aria-label="Check a voice clip">
			<AutoHeight>
				{started ? (
					<div key="run" className="d1-phase">
						<RunPanel
							state={state}
							nodes={nodes}
							result={result}
							file={file}
							onReset={onReset}
						/>
					</div>
				) : verifying ? (
					<div key="verify" className="d1-phase">
						<VerifyPanel
							file={verifying}
							siteKey={turnstileSiteKey}
							onToken={onVerified}
							onCancel={onReset}
						/>
					</div>
				) : (
					<div key="idle" className="d1-phase">
						<ClipInput onFile={onFile} loadSample={loadSample} error={error} />
					</div>
				)}
			</AutoHeight>
			<output className="d1-sr">
				{started ? announce(nodes, state.outcome) : ""}
			</output>
		</section>
	);
}

function announce(
	nodes: ReturnType<typeof nodeStates>,
	outcome: string | null,
) {
	if (outcome === "SUCCEEDED") return "Results ready.";
	if (outcome !== null) return "We couldn't check this clip.";
	const active = STEPS.find((s) => nodes[s.id] === "processing");
	return active ? `${active.label}.` : "";
}
