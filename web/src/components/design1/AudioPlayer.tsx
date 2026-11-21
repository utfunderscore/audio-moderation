import { Button } from "@ui-sdk/react/button";
import {
	Slider,
	SliderControl,
	SliderIndicator,
	SliderThumb,
	SliderTrack,
} from "@ui-sdk/react/slider";
import { useEffect, useRef, useState } from "react";

import { MutedIcon, PauseIcon, PlayIcon, VolumeIcon } from "./icons";

const BARS = 48;
const DEFAULT_VOLUME = 0.2;

function formatTime(seconds: number) {
	if (!Number.isFinite(seconds)) return "0s";
	const s = Math.max(0, Math.ceil(seconds));
	if (s < 60) return `${s}s`;
	return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Peak level per bar, 0–1, decoded from the clip itself. */
function usePeaks(file: File) {
	const [peaks, setPeaks] = useState<number[] | null>(null);

	useEffect(() => {
		let cancelled = false;
		setPeaks(null);
		const context = new AudioContext();
		file
			.arrayBuffer()
			.then((buffer) => context.decodeAudioData(buffer))
			.then((decoded) => {
				const data = decoded.getChannelData(0);
				const size = Math.floor(data.length / BARS);
				const levels = Array.from({ length: BARS }, (_, bar) => {
					let sum = 0;
					for (let i = bar * size; i < (bar + 1) * size; i++) {
						sum += data[i] * data[i];
					}
					return Math.sqrt(sum / size);
				});
				const max = Math.max(...levels) || 1;
				if (!cancelled) setPeaks(levels.map((level) => level / max));
			})
			.catch(() => {})
			.finally(() => void context.close());
		return () => {
			cancelled = true;
		};
	}, [file]);

	return peaks;
}

export function AudioPlayer({ file }: { file: File }) {
	const audio = useRef<HTMLAudioElement>(null);
	const [src, setSrc] = useState<string | null>(null);
	const [playing, setPlaying] = useState(false);
	const [current, setCurrent] = useState(0);
	const [duration, setDuration] = useState(0);
	const [volume, setVolume] = useState(DEFAULT_VOLUME);
	const [muted, setMuted] = useState(false);
	const peaks = usePeaks(file);

	useEffect(() => {
		const el = audio.current;
		if (!el) return;
		el.volume = volume;
		el.muted = muted;
	}, [volume, muted]);

	useEffect(() => {
		const url = URL.createObjectURL(file);
		setSrc(url);
		return () => URL.revokeObjectURL(url);
	}, [file]);

	const toggle = () => {
		const el = audio.current;
		if (!el) return;
		if (el.paused) void el.play();
		else el.pause();
	};

	const progress = duration ? current / duration : 0;
	const started = playing || current > 0;

	return (
		<div className="d1-player" data-playing={playing ? "" : undefined}>
			{src && (
				// biome-ignore lint/a11y/useMediaCaption: user-supplied clip; the transcript below serves as its text alternative
				<audio
					ref={audio}
					src={src}
					preload="metadata"
					onPlay={() => setPlaying(true)}
					onPause={() => setPlaying(false)}
					onEnded={(e) => {
						setPlaying(false);
						e.currentTarget.currentTime = 0;
						setCurrent(0);
					}}
					onTimeUpdate={(e) => setCurrent(e.currentTarget.currentTime)}
					onLoadedMetadata={(e) => {
						e.currentTarget.volume = volume;
						e.currentTarget.muted = muted;
						setDuration(e.currentTarget.duration);
					}}
				/>
			)}
			<Button
				className="d1-player-toggle"
				type="button"
				aria-label={playing ? `Pause ${file.name}` : `Play ${file.name}`}
				onClick={toggle}
			>
				{playing ? <PauseIcon /> : <PlayIcon />}
			</Button>
			<Slider
				className="d1-wave"
				value={current}
				min={0}
				max={duration || 1}
				step={0.1}
				onValueChange={(value) => {
					const next = Array.isArray(value) ? value[0] : value;
					setCurrent(next);
					if (audio.current) audio.current.currentTime = next;
				}}
			>
				<SliderControl>
					<SliderTrack>
						<span className="d1-wave-bars" aria-hidden="true">
							{Array.from({ length: BARS }, (_, i) => (
								<i
									// biome-ignore lint/suspicious/noArrayIndexKey: bars are fixed positions along the clip
									key={i}
									data-played={(i + 0.5) / BARS <= progress ? "" : undefined}
									style={{
										height: `${peaks ? 14 + peaks[i] * 86 : 14}%`,
									}}
								/>
							))}
						</span>
						<SliderThumb aria-label="Seek" />
					</SliderTrack>
				</SliderControl>
			</Slider>
			<span className="d1-player-time">
				{formatTime(started ? duration - current : duration)}
			</span>
			<div className="d1-volume">
				<Button
					className="d1-volume-toggle"
					type="button"
					aria-label={muted || volume === 0 ? "Unmute" : "Mute"}
					onClick={() => {
						if (volume === 0) {
							setVolume(DEFAULT_VOLUME);
							setMuted(false);
						} else {
							setMuted((m) => !m);
						}
					}}
				>
					{muted || volume === 0 ? <MutedIcon /> : <VolumeIcon />}
				</Button>
				<div className="d1-volume-pop">
					<div className="d1-volume-panel">
						<Slider
							className="d1-volume-slider"
							orientation="vertical"
							value={muted ? 0 : Math.round(volume * 100)}
							min={0}
							max={100}
							step={1}
							onValueChange={(value) => {
								const next = Array.isArray(value) ? value[0] : value;
								setVolume(next / 100);
								setMuted(false);
							}}
						>
							<SliderControl>
								<SliderTrack>
									<SliderIndicator />
									<SliderThumb aria-label="Volume" />
								</SliderTrack>
							</SliderControl>
						</Slider>
					</div>
				</div>
			</div>
		</div>
	);
}
