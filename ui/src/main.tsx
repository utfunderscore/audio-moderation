import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import "./index.css"
import { ApiBackend } from "@/api/api-backend"
import { ThemeProvider } from "@/components/theme-provider.tsx"
import App from "./App.tsx"

/**
 * Placeholder backend. The UI depends only on the `Backend` interface in
 * `src/api/backend.ts`; this is the single place the transport is wired to the
 * product UI. Replace it with the real implementation.
 */
const backend = new ApiBackend()

const rootElement = document.getElementById("root")

if (rootElement === null) {
  throw new Error("Root element not found")
}

createRoot(rootElement).render(
  <StrictMode>
    <ThemeProvider defaultTheme="dark">
      <App backend={backend} />
    </ThemeProvider>
  </StrictMode>
)
