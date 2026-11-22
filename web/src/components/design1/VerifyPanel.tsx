import { Button } from "@ui-sdk/react/button";

import { TurnstileWidget } from "./TurnstileWidget";

const TURNSTILE_ACTION = "submit_review";

type VerifyPanelProps = {
	file: File;
	siteKey: string;
	onToken: (token: string) => void;
	onCancel: () => void;
};

export function VerifyPanel({
	file,
	siteKey,
	onToken,
	onCancel,
}: VerifyPanelProps) {
	return (
		<div className="d1-run d1-verify">
			<div className="d1-verdict" data-tone="pending">
				<h2>Quick security check</h2>
				<p>
					Confirm you're human and we'll start checking{" "}
					<span className="d1-verify-file">{file.name}</span>.
				</p>
			</div>
			<TurnstileWidget
				siteKey={siteKey}
				action={TURNSTILE_ACTION}
				onToken={onToken}
			/>
			<div className="d1-next">
				<Button
					className="d1-button"
					data-variant="ghost"
					type="button"
					onClick={onCancel}
				>
					Cancel
				</Button>
			</div>
		</div>
	);
}
