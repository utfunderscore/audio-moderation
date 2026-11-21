## Commands and configuration

Run from `web/`:
- `npm ci` installs the locked dependencies; `npm run dev` binds
  `127.0.0.1:5173` with a strict port.
- `npm test`; focus with `npm test -- src/domain/reducer.test.ts`.
  Vitest uses `environment: "node"` and only includes `src/**/*.test.ts`;
  DOM/component tests need explicit configuration changes.
- `npm run lint`, `npm run format`, `npm run check` use Biome.
- `npm run build` runs Vite only; use `npm run typecheck` (`tsc --noEmit`) for
  TypeScript verification.
- Cloudflare Workers runs the Start server entry (`@cloudflare/vite-plugin`,
  `wrangler.jsonc`). `AWS_PROFILE=admin npm run deploy:dry-run` validates packaging
  and `AWS_PROFILE=admin npm run deploy` publishes. Both deployment commands build
  first. `VITE_*` endpoint URLs must be supplied at build time.

## Backend seam and protocol boundaries

- The UI has a real `ApiBackend` in `src/api/api-backend.ts`; `src/api/backend.ts` declares the
  single `Backend` interface for every operation the UI needs (`listJobs`,
  `startEvaluation`, `subscribeTaskEvents`, `getEvaluationResult`,
  `getJobAudio`). Components and hooks depend only on it; do not add transport
  calls (fetch, WebSocket, uploads, auth) to components, `hooks/`, or
  `domain/`.
- `src/lib/backend.ts` is the composition root and supplies `ApiBackend`. Turnstile's
  browser script is loaded via `src/api/turnstile.ts`; do not put RPC/upload
  transport in components or hooks. New reviews need a single-use Turnstile
  response (`submit_review` action); owner-authenticated idempotent replays do not.
- The deployed protocol first exchanges an authorized evaluation access token for
  a one-time task-events ticket, then subscribes with
  `{"action":"subscribe","ticket":<ticket>}`. Incoming frames are raw UTF-8
  event names, not JSON results. Replay/live delivery can duplicate events, so
  `subscribeTaskEvents` implementations must tolerate duplicates; the stream
  cannot distinguish replay from live frames.
- Transcripts and moderation scores are not delivered on the stream. They come
  from `getEvaluationResult` after the workflow settles.
- Keep lifecycle transitions in `src/domain/reducer.ts`: it seeds from the
  locally returned pending status and handles duplicate/out-of-order events without
  regressing stages. Preserve unknown-event handling when extending the protocol.
- This is a TanStack Start app (file routes in `src/routes/`, root shell in
  `src/routes/__root.tsx`, `src/routeTree.gen.ts` is generated). Routes render on
  the server too: keep `window`/`document`/storage access lazy or in effects.
- `@/` resolves to `src/` in both Vite (tsconfig paths) and Vitest.
- `src/routes/index.tsx` is the landing page: the pipeline demo built from
  `src/components/design1/` and styled by `src/design1.css`. It currently wires
  `SimulatedBackend`; `src/lib/backend.ts` only exports the `ApiBackend`.

## ui-sdk

Before using ui-sdk, follow the `ui-sdk` skill if it's installed, and read
`node_modules/@ui-sdk/react/docs/llms.txt`. To build a new component, also
search `node_modules/@ui-sdk/react/docs/core.md` by heading, and read
`rendering.md` next to it.
