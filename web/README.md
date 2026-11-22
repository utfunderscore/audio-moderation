# web

A minimal TanStack Start app with one route and plain CSS.

```bash
npm install
npm run dev
```

Edit `src/routes/index.tsx` to get started. Add route files under
`src/routes`; TanStack Router updates `src/routeTree.gen.ts` for you.

Build the production app with:

```bash
npm run build
```

## Deploy to Cloudflare Workers

The app runs as a Worker (`@cloudflare/vite-plugin`, `wrangler.jsonc`) on the
`guard.utf.lol` custom domain. The `VITE_*` endpoint and Turnstile site key are
inlined at build time, so they must be present when building.

Preferred, from the repository root (needs the deployed Terraform state):

```bash
CLOUDFLARE_ACCOUNT_ID=<account> AWS_PROFILE=admin ./scripts/deploy-ui.sh
```

This syncs the public Terraform outputs into `web/.env.local` and
`web/.env.production.local`, then runs `npm run deploy`.

Without Terraform state, fill in `web/.env.production.local` from
`.env.example`, then from `web/`:

```bash
AWS_PROFILE=admin npm run deploy:dry-run   # build and package only
AWS_PROFILE=admin npm run deploy           # checks VITE_* values, builds, publishes
```

Wrangler needs Cloudflare credentials: `npx wrangler login`, or
`CLOUDFLARE_API_TOKEN` in non-interactive shells. The Turnstile widget's allowed
hostnames (`turnstile_allowed_hostnames` in Terraform) must include the UI host.
