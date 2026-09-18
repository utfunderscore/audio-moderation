# Running the demo over Tailscale Funnel

This exposes the local Vite dev server on the public internet at
`https://<machine>.<tailnet>.ts.net/`. Vite is already configured for it: the
`dev:funnel` script enables Tailscale-aware HMR and allows the `*.ts.net` host.

Only do this when you actually want the URL reachable from outside your tailnet.
Funnel is public; anyone with the URL can open the demo.

## Prerequisites

- Tailscale installed and signed in: `tailscale up`.
- HTTPS certificates enabled for the tailnet (admin console → **DNS** → HTTPS
  Certificates). Funnel needs a valid `*.ts.net` certificate.
- Funnel permitted for this device. The first `tailscale funnel` run prints a
  link to enable it (admin console → **Access controls**, node attribute
  `funnel`).
- Funnel only listens on ports **443**, **8443**, and **10000**.

## Commands

Terminal 1 — start the dev server with funnel-aware settings:

```sh
cd ui
npm run dev:funnel          # same as TAILSCALE_FUNNEL=1 vite
```

Terminal 2 — expose it (443 → 127.0.0.1:5173) and read the public URL:

```sh
tailscale funnel --bg 5173          # add --yes to skip the confirmation prompt
tailscale funnel status             # prints https://<machine>.<tailnet>.ts.net/
```

Open the URL it prints. Stop exposing when you're done:

```sh
tailscale funnel reset              # remove the funnel config
# then Ctrl-C the dev server
```

## Variants

Tailnet-only instead of public (same URL, reachable only by your devices):

```sh
tailscale serve --bg 5173
tailscale serve status
tailscale serve reset
```

Different public port (keep the two in sync):

```sh
TAILSCALE_FUNNEL_PORT=8443 npm run dev:funnel
tailscale funnel --bg --https=8443 5173
```

Serve a production build instead of the dev server (no HMR involved):

```sh
npm run build
npm run preview                     # 127.0.0.1:4173
tailscale funnel --bg 4173
```

Sub-path exposure (needs a matching Vite `base`, e.g. `base: "/demo/"`):

```sh
tailscale funnel --bg --set-path /demo 5173
```

## What the config does

`ui/vite.config.ts`:

- `server.allowedHosts: [".ts.net"]` — without this Vite rejects the funnel's
  `Host` header with "Blocked request. This host is not allowed."
- `hmr: { protocol: "wss", clientPort: 443 }` when `TAILSCALE_FUNNEL=1` — the
  HMR websocket connects back over the public 443 endpoint instead of the local
  dev port 5173.
- `port: 5173` / `strictPort: true` — keeps the funnel target stable.

## Troubleshooting

- **"Blocked request. This host is not allowed."** — the dev server was started
  without the config change taking effect; restart it.
- **HMR websocket fails / page doesn't hot-reload** — confirm you started with
  `npm run dev:funnel`, and that the funnel port matches `TAILSCALE_FUNNEL_PORT`
  (default 443).
- **Funnel reports HTTPS is not enabled** — enable HTTPS certificates for the
  tailnet in the admin console, then retry.
- **Funnel reports it is not permitted** — enable it for the node in the admin
  console, then retry.
- **Port rejected** — choose 443, 8443, or 10000.
