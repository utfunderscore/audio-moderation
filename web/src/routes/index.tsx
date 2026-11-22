import { createFileRoute } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";

import { getSampleAudio } from "@/api/sample-audio";
import { Design1Topbar } from "@/components/design1/Design1Topbar";
import { PipelineDemo } from "@/components/design1/PipelineDemo";
import { PixelStars } from "@/components/PixelStars";
import { DEMO_USER_ID } from "@/domain/jobs";
import { useEvaluation } from "@/hooks/useEvaluation";
import { backend } from "@/lib/backend";
import design1Css from "../design1.css?url";

const TURNSTILE_SITE_KEY = import.meta.env.VITE_TURNSTILE_SITE_KEY ?? "";
const MISSING_SITE_KEY =
	"Submission is unavailable: VITE_TURNSTILE_SITE_KEY is not configured.";

export const Route = createFileRoute("/")({
	head: () => ({
		meta: [{ title: "socialguard · Catch abuse in voice chat" }],
		links: [{ rel: "stylesheet", href: design1Css }],
	}),
	component: LandingPage,
});

/** Content the background stars must never sit behind. */
const STAR_FREE_ZONES =
	".d1-title, .d1-lede, .d1-card, .d1-fineprint, .d1-footer";

function LandingPage() {
	const evaluation = useEvaluation(backend);
	const { resume } = evaluation;
	const [file, setFile] = useState<File | null>(null);
	const [verifying, setVerifying] = useState<File | null>(null);
	const [configError, setConfigError] = useState<string | null>(null);
	const tokenUsed = useRef(false);

	// Pick an in-flight job back up after a reload; access tokens live in this tab's sessionStorage.
	useEffect(() => {
		let cancelled = false;
		void backend.listJobs(DEMO_USER_ID).then((jobs) => {
			const job = jobs.find((candidate) => candidate.status === "processing");
			if (cancelled || job === undefined) return;
			resume({ evaluationId: job.id, startedAt: job.submittedAt });
			void backend
				.getJobAudio(job.id)
				.then((blob) => {
					if (!cancelled) {
						setFile(new File([blob], job.fileName, { type: blob.type }));
					}
				})
				.catch(() => {});
		});
		return () => {
			cancelled = true;
		};
	}, [resume]);

	const requestVerification = (audio: File) => {
		if (!TURNSTILE_SITE_KEY) {
			setConfigError(MISSING_SITE_KEY);
			return;
		}
		setConfigError(null);
		tokenUsed.current = false;
		evaluation.reset();
		setFile(null);
		setVerifying(audio);
	};

	// Turnstile responses are single-use, so submit at most once per challenge.
	const submit = (turnstileToken: string) => {
		if (verifying === null || tokenUsed.current) return;
		tokenUsed.current = true;
		setFile(verifying);
		setVerifying(null);
		evaluation.start({
			userId: DEMO_USER_ID,
			audio: verifying,
			turnstileToken,
		});
	};

	const reset = () => {
		setFile(null);
		setVerifying(null);
		evaluation.reset();
	};

	const submissionError =
		evaluation.submissionError === null
			? null
			: `${evaluation.submissionError} Try again with a new security check.`;

	return (
		<div className="d1">
			<Design1Topbar />
			<div className="d1-body">
				<PixelStars avoid={STAR_FREE_ZONES} />
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
						verifying={verifying}
						turnstileSiteKey={TURNSTILE_SITE_KEY}
						error={configError ?? submissionError}
						onFile={requestVerification}
						onVerified={submit}
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
		</div>
	);
}
