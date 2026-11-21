import type { StageId } from "@/domain/stages";

import { type NodeState, STEPS } from "./pipeline-view";

type StageTrackProps = {
	nodes: Record<StageId, NodeState>;
	collapsed: boolean;
};

export function StageTrack({ nodes, collapsed }: StageTrackProps) {
	const current = STEPS.find(
		(step) => nodes[step.id] === "processing" || nodes[step.id] === "queued",
	);
	return (
		<div
			className="d1-track-wrap"
			data-collapsed={collapsed ? "" : undefined}
			inert={collapsed}
			aria-hidden={collapsed || undefined}
		>
			<div className="d1-track-inner">
				<ol className="d1-track" aria-label="Progress">
					{STEPS.map((step) => {
						const node = nodes[step.id];
						const active = node === "processing" || node === "queued";
						return (
							<li
								key={step.id}
								className="d1-step"
								data-state={active ? "active" : node}
								aria-current={active ? "step" : undefined}
							>
								<span className="d1-step-bar" aria-hidden="true">
									<i />
								</span>
								<span className="d1-step-label">{step.label}</span>
							</li>
						);
					})}
				</ol>
				<p className="d1-track-caption" aria-hidden="true">
					{current ? `${current.label}…` : ""}
				</p>
			</div>
		</div>
	);
}
