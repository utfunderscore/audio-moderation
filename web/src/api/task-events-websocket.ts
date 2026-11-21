import type { TaskEventHandlers, Unsubscribe } from "@/api/backend"

export function subscribeTaskEventsWebSocket(
  endpoint: string,
  createTicket: (signal: AbortSignal) => Promise<string>,
  handlers: TaskEventHandlers
): Unsubscribe {
  const controller = new AbortController()
  let socket: WebSocket | undefined
  let failed = false

  const fail = (error: unknown) => {
    if (controller.signal.aborted || failed) return
    failed = true
    console.debug("[Task events] failed", {
      errorType: error instanceof Error ? error.name : typeof error,
    })
    handlers.onConnectionChange?.("error")
    handlers.onError?.(error)
  }

  handlers.onConnectionChange?.("connecting")
  console.debug("[Task events] connecting")
  void (async () => {
    if (endpoint === "") {
      throw new Error("VITE_TASK_EVENTS_ENDPOINT is not configured")
    }
    console.debug("[Task events] requesting ticket")
    const ticket = await createTicket(controller.signal)
    if (controller.signal.aborted) {
      console.debug("[Task events] setup cancelled")
      return
    }
    if (ticket === "") {
      throw new Error("CreateTaskEventsTicket returned an empty ticket")
    }

    console.debug("[Task events] ticket received; opening WebSocket")
    const connection = new WebSocket(endpoint)
    socket = connection
    connection.onopen = () => {
      if (controller.signal.aborted) return
      console.debug("[Task events] WebSocket open")
      connection.send(JSON.stringify({ action: "subscribe", ticket }))
      console.debug("[Task events] subscribe sent")
      handlers.onConnectionChange?.("subscribed")
    }
    connection.onmessage = (event: MessageEvent) => {
      if (controller.signal.aborted || failed) return
      if (typeof event.data === "string") {
        console.debug("[Task events] event", event.data)
        handlers.onFrame({ name: event.data })
      }
    }
    connection.onerror = () => fail(new Error("Task-events WebSocket failed"))
    connection.onclose = (event) => {
      console.debug("[Task events] WebSocket closed", {
        code: event.code,
        wasClean: event.wasClean,
        unsubscribed: controller.signal.aborted,
      })
      if (controller.signal.aborted || failed) return
      if (!event.wasClean) {
        fail(
          new Error(`Task-events WebSocket closed unexpectedly (${event.code})`)
        )
      } else {
        handlers.onConnectionChange?.("closed")
      }
    }
  })().catch(fail)

  return () => {
    console.debug("[Task events] unsubscribe")
    controller.abort()
    socket?.close()
  }
}
