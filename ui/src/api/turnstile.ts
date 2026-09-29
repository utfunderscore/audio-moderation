/** Cloudflare's explicitly rendered browser widget. No response token is stored here. */
export interface TurnstileApi {
  render(
    container: HTMLElement,
    options: {
      sitekey: string
      action: string
      callback: (token: string) => void
      "expired-callback": () => void
      "error-callback": () => void
      "timeout-callback": () => void
      "response-field": false
    }
  ): string
  remove(widgetId: string): void
}

const SCRIPT_URL =
  "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit"
let loading: Promise<TurnstileApi> | undefined

function api(): TurnstileApi | undefined {
  return (window as Window & { turnstile?: TurnstileApi }).turnstile
}

export function loadTurnstile(): Promise<TurnstileApi> {
  const existing = api()
  if (existing !== undefined) return Promise.resolve(existing)
  if (loading !== undefined) return loading

  loading = new Promise<TurnstileApi>((resolve, reject) => {
    const script = document.createElement("script")
    script.src = SCRIPT_URL
    script.async = true
    script.onload = () => {
      const loaded = api()
      if (loaded === undefined) {
        reject(new Error("Security check failed to initialize"))
      } else {
        resolve(loaded)
      }
    }
    script.onerror = () => reject(new Error("Security check could not load"))
    document.head.append(script)
  }).catch((error: unknown) => {
    loading = undefined
    document.querySelector(`script[src="${SCRIPT_URL}"]`)?.remove()
    throw error
  })
  return loading
}
