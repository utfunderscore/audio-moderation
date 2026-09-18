import type { ConnectionState } from "@/domain/reducer"
import type { TaskEventsClient, TaskFrame } from "./client"

export interface WebSocketTaskEventsOptions {
  /** `wss://` stage URL from the `pipeline_task_events_websocket_endpoint` output. */
  endpoint: string
  onDecodeError?: (error: unknown) => void
}

/**
 * Real transport adapter, not wired into the demo yet.
 *
 * Handshake (see task-events-lambda/src/lib.rs):
 * 1. Open `wss://<stage>`.
 * 2. Send `{"action":"subscribe","taskId":<number>}`. API Gateway's route
 *    selection expression is `$request.body.action`; the handler reads `taskId`.
 * 3. Receive raw UTF-8 event-name frames. They may arrive as text or binary.
 *    Server-side replay-then-live cannot be distinguished per frame, so every
 *    frame is tagged `live`; the reducer already dedupes by name.
 *
 * Delivery is at-least-once: callers must tolerate duplicates at the
 * replay/live boundary, exactly like `MockTaskEventsClient`.
 */
export class WebSocketTaskEventsClient implements TaskEventsClient {
  private readonly endpoint: string
  private readonly onDecodeError: (error: unknown) => void
  private readonly frameHandlers = new Set<(frame: TaskFrame) => void>()
  private readonly connectionHandlers = new Set<
    (state: ConnectionState) => void
  >()
  private socket: WebSocket | null = null
  private connection: ConnectionState = "idle"

  constructor(options: WebSocketTaskEventsOptions) {
    this.endpoint = options.endpoint
    this.onDecodeError = options.onDecodeError ?? (() => {})
  }

  connect(): Promise<void> {
    return new Promise((resolve, reject) => {
      const socket = new WebSocket(this.endpoint)
      socket.binaryType = "arraybuffer"
      this.socket = socket
      this.setConnection("connecting")

      socket.onopen = () => {
        this.setConnection("subscribed")
        resolve()
      }
      socket.onmessage = (event) => {
        this.handleMessage(event)
      }
      socket.onerror = () => {
        this.setConnection("error")
        reject(new Error("task-events socket failed"))
      }
      socket.onclose = () => {
        this.setConnection("closed")
      }
    })
  }

  async subscribe(taskId: number): Promise<void> {
    if (this.socket === null || this.socket.readyState !== WebSocket.OPEN) {
      throw new Error("connect before subscribe")
    }
    this.socket.send(JSON.stringify({ action: "subscribe", taskId }))
  }

  close(): void {
    this.socket?.close()
    this.socket = null
  }

  onFrame(handler: (frame: TaskFrame) => void): () => void {
    this.frameHandlers.add(handler)
    return () => {
      this.frameHandlers.delete(handler)
    }
  }

  onConnectionChange(handler: (state: ConnectionState) => void): () => void {
    this.connectionHandlers.add(handler)
    return () => {
      this.connectionHandlers.delete(handler)
    }
  }

  private setConnection(state: ConnectionState): void {
    if (this.connection === state) return
    this.connection = state
    for (const handler of this.connectionHandlers) handler(state)
  }

  private handleMessage(event: MessageEvent): void {
    try {
      const name =
        typeof event.data === "string"
          ? event.data
          : event.data instanceof ArrayBuffer
            ? new TextDecoder().decode(event.data)
            : null
      if (name === null || name.trim() === "") return
      for (const handler of this.frameHandlers) {
        handler({ name: name.trim(), source: "live" })
      }
    } catch (error) {
      this.onDecodeError(error)
    }
  }
}
