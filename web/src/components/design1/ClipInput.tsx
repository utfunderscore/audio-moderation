import { Button } from "@ui-sdk/react/button";
import {
	FileInput,
	FileInputDropzone,
	FileInputTrigger,
} from "@ui-sdk/react/file-input";
import { useEffect, useRef, useState } from "react";

import type { AudioSample } from "@/api/sample-audio";
import { DotRipple, type DotRippleHandle } from "@/components/DotRipple";

import { UploadIcon } from "./icons";

const SAMPLES: { sample: AudioSample; title: string; detail: string }[] = [
	{ sample: "abusive", title: "Ranked match", detail: "heated · 21s" },
	{ sample: "neutral", title: "Neutral clip", detail: "calm · 10s" },
];

type ClipInputProps = {
	onFile: (file: File) => void;
	loadSample: (sample: AudioSample, signal: AbortSignal) => Promise<File>;
	error: string | null;
};

// Matches the stylesheet breakpoint that hides the dot grid.
const COMPACT_QUERY = "(max-width: 600px)";
const PULSE_MS = 1600;
const HANDOFF_MS = 750;

export function ClipInput({ onFile, loadSample, error }: ClipInputProps) {
	const [message, setMessage] = useState<string | null>(null);
	const [loading, setLoading] = useState<AudioSample | null>(null);
	const isMac = useIsMac();
	const hole = useRef<HTMLDivElement>(null);
	const ripple = useRef<DotRippleHandle>(null);
	const handoff = useRef<ReturnType<typeof setTimeout> | null>(null);
	const [handingOff, setHandingOff] = useState(false);

	useEffect(
		() => () => {
			if (handoff.current) clearTimeout(handoff.current);
		},
		[],
	);

	// Fire a green pulse from the upload button, then hand the file over once
	// it has swept the grid; the card swaps to the result view on handoff.
	const submit = (file: File) => {
		if (handoff.current) return;
		const button = hole.current?.querySelector(".d1-cta");
		if (
			!button ||
			matchMedia("(prefers-reduced-motion: reduce)").matches ||
			matchMedia(COMPACT_QUERY).matches
		) {
			onFile(file);
			return;
		}
		const b = button.getBoundingClientRect();
		ripple.current?.pulse(b.left + b.width / 2, b.top + b.height / 2, {
			color: "var(--d1-accent)",
			brightness: 90,
			duration: PULSE_MS,
		});
		setHandingOff(true);
		handoff.current = setTimeout(() => onFile(file), HANDOFF_MS);
	};

	const trySample = async (sample: AudioSample) => {
		setMessage(null);
		setLoading(sample);
		try {
			submit(await loadSample(sample, AbortSignal.timeout(15_000)));
		} catch (cause) {
			setMessage(
				cause instanceof Error ? cause.message : "Unable to load sample.",
			);
		} finally {
			setLoading(null);
		}
	};

	const shown = message ?? error;

	return (
		<div className="d1-input">
			<FileInput
				accept="audio/*"
				onValueChange={([file]) => {
					setMessage(null);
					if (file) submit(file);
				}}
				onReject={() => setMessage("That isn't an audio file.")}
			>
				<FileInputDropzone
					className="d1-drop"
					target="document"
					pasteTarget="document"
				>
					{({ dragging, dragReject }) => (
						<>
							<DotRipple
								handle={ripple}
								holeRef={hole}
								edgeFade={24}
								holeFade={80}
							/>
							<div ref={hole} className="d1-drop-content">
								<FileInputTrigger className="d1-cta">
									<UploadIcon />
									Upload audio
								</FileInputTrigger>
								<p className="d1-drop-hint">
									{dragReject ? (
										"Only audio files are supported"
									) : dragging ? (
										"Release to moderate this clip"
									) : (
										<>
											or drop a file anywhere, or paste with{" "}
											<span className="d1-keys">
												{isMac ? (
													<>
														<kbd>⌘</kbd>
														<kbd>V</kbd>
													</>
												) : (
													<>
														<kbd>Ctrl</kbd>+<kbd>V</kbd>
													</>
												)}
											</span>
										</>
									)}
								</p>
							</div>
						</>
					)}
				</FileInputDropzone>
			</FileInput>

			{shown && (
				<p className="d1-input-error" role="alert">
					{shown}
				</p>
			)}

			<div className="d1-samples">
				<p className="d1-samples-label">No clip? Try one of these:</p>
				<div className="d1-samples-row">
					{SAMPLES.map(({ sample, title, detail }) => (
						<Button
							key={sample}
							className="d1-sample"
							type="button"
							disabled={loading !== null || handingOff}
							onClick={() => trySample(sample)}
						>
							<span className="d1-sample-wave" aria-hidden="true">
								{Array.from({ length: 14 }, (_, i) => (
									<i
										// biome-ignore lint/suspicious/noArrayIndexKey: static decorative bars
										key={i}
										style={{ height: `${barHeight(sample, i)}%` }}
									/>
								))}
							</span>
							<span className="d1-sample-text">
								<span>{title}</span>
								<span className="d1-sample-detail">
									{loading === sample ? "loading…" : detail}
								</span>
							</span>
						</Button>
					))}
				</div>
			</div>
		</div>
	);
}

function barHeight(sample: AudioSample, i: number) {
	const seed = sample === "abusive" ? 7 : 3;
	return 25 + ((Math.sin(i * seed) + 1) / 2) * 75;
}

// Resolved after mount so server markup and hydration agree.
function useIsMac() {
	const [isMac, setIsMac] = useState(false);
	useEffect(() => {
		const nav = navigator as Navigator & {
			userAgentData?: { platform?: string };
		};
		const platform = nav.userAgentData?.platform || nav.platform || "";
		if (/mac|iphone|ipad|ipod/i.test(platform)) setIsMac(true);
	}, []);
	return isMac;
}
