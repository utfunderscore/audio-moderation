import { useEffect, useRef, useState } from "react"
import { loadTurnstile, type TurnstileApi } from "@/api/turnstile"

interface TurnstileWidgetProps {
  siteKey: string
  action: string
  onToken: (token: string) => void
}

export function TurnstileWidget({
  siteKey,
  action,
  onToken,
}: TurnstileWidgetProps) {
  const container = useRef<HTMLDivElement>(null)
  const widget = useRef<{ api: TurnstileApi; id: string } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [reloadKey, setReloadKey] = useState(0)

  // biome-ignore lint/correctness/useExhaustiveDependencies: reloadKey explicitly retries failed widget initialization.
  useEffect(() => {
    let cancelled = false
    const element = container.current
    if (element === null) return
    void loadTurnstile()
      .then((api) => {
        if (cancelled) return
        const id = api.render(element, {
          sitekey: siteKey,
          action,
          callback: (token) => {
            if (cancelled) return
            onToken(token)
            setError(null)
          },
          "expired-callback": () => {
            if (!cancelled)
              setError("Security check expired. Please try again.")
          },
          "error-callback": () => {
            if (cancelled) return
            setError("Security check failed. Please try again.")
          },
          "timeout-callback": () => {
            if (!cancelled)
              setError("Security check timed out. Please try again.")
          },
          "response-field": false,
        })
        widget.current = { api, id }
        setError(null)
      })
      .catch((cause: unknown) => {
        if (!cancelled) {
          setError(
            cause instanceof Error
              ? cause.message
              : "Security check could not load"
          )
        }
      })
    return () => {
      cancelled = true
      if (widget.current !== null) {
        widget.current.api.remove(widget.current.id)
        widget.current = null
      }
    }
  }, [siteKey, action, reloadKey, onToken])

  return (
    <div className="mx-auto flex w-full max-w-xl flex-col items-center gap-2">
      <div ref={container} />
      {error !== null ? (
        <div className="text-center text-xs text-destructive" role="alert">
          {error}{" "}
          <button
            type="button"
            className="underline"
            onClick={() => setReloadKey((value) => value + 1)}
          >
            Retry security check
          </button>
        </div>
      ) : null}
    </div>
  )
}
