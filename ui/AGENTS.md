## Commands and configuration

Run from `ui/`:
- `npm ci` installs the locked dependencies; `npm run dev` binds
  `127.0.0.1:5173` with a strict port.
- `npm test`; focus with `npm test -- src/domain/reducer.test.ts`.
  Vitest uses `environment: "node"` and only includes `src/**/*.test.ts`;
  DOM/component tests need explicit configuration changes.
- `npm run lint` and `npm run format` use Biome, despite the ESLint/Prettier
  configs also present. `npm run check:fix` applies Biome fixes.
- `npm run build` runs `tsc -b` then Vite. Use it for TypeScript verification;
  `npm run typecheck` runs `tsc --noEmit` against an empty root files list,
  rather than building the referenced app/node projects.
- Public Funnel/HMR uses `npm run dev:funnel`.
- Cloudflare Workers hosts `dist/` using Static Assets and SPA fallback in
  `wrangler.jsonc`. `npm run preview:workers` builds and runs local Wrangler on
  `127.0.0.1:8787`; `AWS_PROFILE=admin npm run deploy:dry-run` validates packaging
  and `AWS_PROFILE=admin npm run deploy` publishes. Both deployment commands build
  first. `VITE_*` endpoint URLs must be supplied at build time.

## Backend seam and protocol boundaries

- The UI contains no backend interaction code. `src/api/backend.ts` declares the
  single `Backend` interface for every operation the UI needs (`listJobs`,
  `startEvaluation`, `subscribeTaskEvents`, `getEvaluationResult`,
  `getJobAudio`). Components and hooks depend only on it; do not add transport
  calls (fetch, WebSocket, uploads, auth) to `components/`, `hooks/`, or
  `domain/`.
- `src/main.tsx` is the composition root and currently passes a placeholder
  `Backend` that throws for every operation. Wire the real implementation there.
  The app builds and renders without one but performs no backend work.
- The deployed protocol first exchanges an authorized evaluation access token for
  a one-time task-events ticket, then subscribes with
  `{"action":"subscribe","ticket":<ticket>}`. Incoming frames are raw UTF-8
  event names, not JSON results. Replay/live delivery can duplicate events, so
  `subscribeTaskEvents` implementations must tolerate duplicates; the stream
  cannot distinguish replay from live frames.
- Transcripts and moderation scores are not delivered on the stream. Read the
  transcript with `getEvaluationResult` after ASR finishes, and refresh the
  result after the workflow settles to get scores.
- Keep lifecycle transitions in `src/domain/reducer.ts`: it seeds from the
  StartEvaluation status and handles duplicate/out-of-order events without
  regressing stages. Preserve unknown-event handling when extending the protocol.
- `@/` resolves to `src/` in both Vite and Vitest. Vendored shadcn components
  live in `src/components/ui/`; demo-specific views live in `components/demo/`.
  Design context is in `DEMO_PLAN.md` and `DEMO_DESIGN_ROUTE.md`.
