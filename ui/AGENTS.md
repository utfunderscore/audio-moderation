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
- Public Funnel/HMR uses `npm run dev:funnel`; follow `TAILSCALE_FUNNEL.md`.
  Ordinary dev mode does not configure the public WSS HMR port.

## Demo and protocol boundaries

- This is a browser-only simulation. `src/hooks/usePipelineRun.ts` constructs
  `MockTaskEventsClient`; selecting a file does not upload it or start AWS work.
  Transcripts and moderation scores come from `src/transport/scenarios.ts`.
- `src/transport/client.ts` defines the transport interface.
  The deployed protocol first exchanges an authorized evaluation access token for
  a one-time task-events ticket, then subscribes with
  `{"action":"subscribe","ticket":<ticket>}`. Incoming frames are raw UTF-8
  event names, not JSON results; replay/live delivery can duplicate events.
  `webSocketClient.ts` is unwired and stale: it still sends `taskId`, cannot mint a
  ticket, and therefore cannot subscribe to the deployed server. It also cannot
  distinguish replay from live frames.
- Keep lifecycle transitions in `src/domain/reducer.ts`: it seeds from the
  StartEvaluation status and handles duplicate/out-of-order events without
  regressing stages. Preserve unknown-event handling when extending the protocol.
- `@/` resolves to `src/` in both Vite and Vitest. Vendored shadcn components
  live in `src/components/ui/`; demo-specific views live in `components/demo/`.
  Design context is in `DEMO_PLAN.md` and `DEMO_DESIGN_ROUTE.md`.
