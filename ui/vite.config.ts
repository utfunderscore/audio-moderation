import path from "path"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"

// Tailscale Funnel terminates TLS on 443 and proxies to the local Vite port, so
// the page is served from https://<machine>.<tailnet>.ts.net/. The HMR socket
// must then reconnect over wss on the public port instead of the local dev
// port. Set TAILSCALE_FUNNEL=1 (or use `npm run dev:funnel`) to enable it.
// Override the public port when the funnel uses something other than 443.
const funnel = process.env.TAILSCALE_FUNNEL === "1"
const funnelPort = Number(process.env.TAILSCALE_FUNNEL_PORT ?? 443)

// MagicDNS names end in .ts.net; a leading dot matches any subdomain.
const tailscaleHosts = [".ts.net"]

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "./src"),
    },
  },
  server: {
    // Tailscale Serve/Funnel dial 127.0.0.1, so bind IPv4 explicitly instead
    // of letting the default `localhost` resolve to ::1 only.
    host: "127.0.0.1",
    // A fixed port keeps the funnel target stable across restarts.
    port: 5173,
    strictPort: true,
    allowedHosts: tailscaleHosts,
    hmr: funnel ? { protocol: "wss", clientPort: funnelPort } : undefined,
  },
  preview: {
    host: "127.0.0.1",
    port: 4173,
    strictPort: true,
    allowedHosts: tailscaleHosts,
  },
})
