import type { ConnectionState, EventSource } from "@/domain/reducer"

/** One task-events frame: the raw event name, tagged with how it arrived. */
export interface TaskFrame {
  name: string
  source: EventSource
}

/**
 * The single seam between the demo UI and the task-events WebSocket.
 *
 * `MockTaskEventsClient` implements this today; `WebSocketTaskEventsClient`
 * documents the real handshake and can be dropped in without UI changes.
 */
export interface TaskEventsClient {
  connect(): Promise<void>
  subscribe(taskId: number): Promise<void>
  close(): void
  onFrame(handler: (frame: TaskFrame) => void): () => void
  onConnectionChange(handler: (state: ConnectionState) => void): () => void
}
