import { Button } from "@ui-sdk/react/button";

import { GitHubIcon } from "./icons";

const SOURCE_URL = "https://github.com/utfunderscore/audio-moderation";

export function Design1Topbar() {
	return (
		<header className="d1-topbar">
			<nav className="d1-topbar-inner" aria-label="Main">
				<a href="/" className="d1-brand font-socialguard">
					socialguard
				</a>
				<div className="d1-topbar-actions">
					<a
						className="d1-icon-link"
						href={SOURCE_URL}
						target="_blank"
						rel="noreferrer"
						aria-label="View source on GitHub"
					>
						<GitHubIcon />
					</a>
					<Button className="d1-button" data-variant="outline" type="button">
						API docs
					</Button>
				</div>
			</nav>
		</header>
	);
}
