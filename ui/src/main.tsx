import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import "./index.css"
import { ApiBackend } from "@/api/api-backend"
import { ThemeProvider } from "@/components/theme-provider.tsx"
import App from "./App.tsx"

/** The composition root wires the real transport to the product UI. */
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
