import { createFileRoute } from "@tanstack/react-router";
import { useState } from "react";

import { getSampleAudio } from "@/api/sample-audio";
import { SimulatedBackend } from "@/api/simulated-backend";
import { Design1Topbar } from "@/components/design1/Design1Topbar";
import { PipelineDemo } from "@/components/design1/PipelineDemo";
import { DEMO_USER_ID } from "@/domain/jobs";
import { useEvaluation } from "@/hooks/useEvaluation";
import design1Css from "../design1.css?url";

export const Route = createFileRoute("/")({
	head: () => ({
		meta: [{ title: "socialguard · Catch abuse in voice chat" }],
		links: [{ rel: "stylesheet", href: design1Css }],
	}),
	component: LandingPage,
});

function LandingPage() {
	const [backend] = useState(() => new SimulatedBackend());
	const evaluation = useEvaluation(backend);
	const [file, setFile] = useState<File | null>(null);

	const start = (audio: File) => {
		setFile(audio);
		evaluation.start({
			userId: DEMO_USER_ID,
			audio,
			turnstileToken: "simulated",
		});
	};

	const reset = () => {
		setFile(null);
		evaluation.reset();
	};

	return (
		<div className="d1">
			<Design1Topbar />
			<main className="d1-main">
				<div className="d1-hero">
					<h1 className="d1-title">
						Catch abuse in voice chat.
						<span> Automatically.</span>
					</h1>
					<p className="d1-lede">
						Upload a voice clip and see what was said, and whether it crosses
						the line, in seconds.
					</p>
				</div>

				<PipelineDemo
					evaluation={evaluation}
					file={file}
					onFile={start}
					onReset={reset}
					loadSample={getSampleAudio}
				/>

				<p className="d1-fineprint">
					Free to try. No account needed. Works with MP3, WAV and M4A.
				</p>
			</main>
			<footer className="d1-footer">
				<div className="d1-footer-inner">
					<span>
						© {new Date().getFullYear()}{" "}
						<span className="font-socialguard">socialguard</span>. All rights
						reserved.
					</span>
					<nav className="d1-footer-links" aria-label="Footer">
						<a href="#privacy">Privacy</a>
						<a href="#terms">Terms</a>
						<a
							href="https://github.com/utfunderscore/audio-moderation"
							target="_blank"
							rel="noreferrer"
						>
							GitHub
						</a>
					</nav>
				</div>
			</footer>
		</div>
	);
}
