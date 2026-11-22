// VITE_* values are inlined at build time, so a deploy without them ships a page
// that cannot submit. Resolve them exactly as `vite build` (production mode) does.
import { loadEnv } from "vite";

const env = loadEnv("production", process.cwd(), "VITE_");

const checks = [
	["VITE_API_ENDPOINT", /^https:\/\/\S+$/],
	["VITE_TASK_EVENTS_ENDPOINT", /^wss:\/\/\S+$/],
	["VITE_TURNSTILE_SITE_KEY", /^[A-Za-z0-9_-]{1,64}$/],
];

const problems = checks
	.filter(([name, pattern]) => !pattern.test(env[name] ?? ""))
	.map(([name]) => name);

if (problems.length > 0) {
	console.error(
		`Refusing to deploy: missing or invalid ${problems.join(", ")}.\n` +
			"Run AWS_PROFILE=admin ../scripts/deploy-ui.sh (syncs them from Terraform),\n" +
			"or set them in web/.env.production.local (see .env.example).",
	);
	process.exit(1);
}
console.log("Deploy environment OK.");
