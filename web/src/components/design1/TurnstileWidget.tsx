import { Button } from "@ui-sdk/react/button";
import { useEffect, useRef, useState } from "react";

import { loadTurnstile, type TurnstileApi } from "@/api/turnstile";

type TurnstileWidgetProps = {
	siteKey: string;
	action: string;
	onToken: (token: string) => void;
};

export function TurnstileWidget({
	siteKey,
	action,
	onToken,
}: TurnstileWidgetProps) {
	const container = useRef<HTMLDivElement>(null);
	const widget = useRef<{ api: TurnstileApi; id: string } | null>(null);
	// Read through a ref so a new callback identity doesn't re-issue the challenge.
	const tokenHandler = useRef(onToken);
	tokenHandler.current = onToken;
	const [error, setError] = useState<string | null>(null);
	const [attempt, setAttempt] = useState(0);

	// biome-ignore lint/correctness/useExhaustiveDependencies: `attempt` re-renders the widget on retry.
	useEffect(() => {
		let cancelled = false;
		const element = container.current;
		if (element === null) return;
		void loadTurnstile()
			.then((api) => {
				if (cancelled) return;
				const id = api.render(element, {
					sitekey: siteKey,
					action,
					theme: "dark",
					callback: (token) => {
						if (cancelled) return;
						setError(null);
						tokenHandler.current(token);
					},
					"expired-callback": () => {
						if (!cancelled) setError("The security check expired.");
					},
					"error-callback": () => {
						if (!cancelled) setError("The security check failed.");
					},
					"timeout-callback": () => {
						if (!cancelled) setError("The security check timed out.");
					},
					"response-field": false,
				});
				widget.current = { api, id };
				setError(null);
			})
			.catch((cause: unknown) => {
				if (cancelled) return;
				setError(
					cause instanceof Error
						? cause.message
						: "The security check could not load.",
				);
			});
		return () => {
			cancelled = true;
			if (widget.current !== null) {
				widget.current.api.remove(widget.current.id);
				widget.current = null;
			}
		};
	}, [siteKey, action, attempt]);

	return (
		<div className="d1-turnstile">
			<div ref={container} />
			{error !== null && (
				<p className="d1-input-error" role="alert">
					{error}{" "}
					<Button
						className="d1-link-button"
						type="button"
						onClick={() => setAttempt((n) => n + 1)}
					>
						Try again
					</Button>
				</p>
			)}
		</div>
	);
}
