import type { ConnectionState } from "@/domain/reducer"
import type { TaskEventsClient, TaskFrame } from "./client"
import { replayableStepCount, type Scenario } from "./scenarios"

/**
 * Plays a scenario script with the same semantics as the deployed transport:
 * subscribe replays a prefix of durable history, then live frames arrive in
 * emission order, duplicates included.
 */
export class MockTaskEventsClient implements TaskEventsClient {
  private readonly scenario: Scenario
  private readonly getSpeed: () => number
  private readonly lateSubscribe: boolean
  private readonly frameHandlers = new Set<(frame: TaskFrame) => void>()
  private readonly connectionHandlers = new Set<
    (state: ConnectionState) => void
  >()
  private readonly timers = new Set<ReturnType<typeof setTimeout>>()
  private connection: ConnectionState = "idle"
  private closed = false

  constructor(
    scenario: Scenario,
    getSpeed: () => number,
    lateSubscribe: boolean
  ) {
    this.scenario = scenario
    this.getSpeed = getSpeed
    this.lateSubscribe = lateSubscribe
  }

  async connect(): Promise<void> {
    this.setConnection("connecting")
    await wait(Math.max(0, 220 / this.getSpeed()))
    if (this.closed) return
    this.setConnection("subscribed")
  }

  async subscribe(taskId: number): Promise<void> {
    // The real transport needs the task id; the mock plays a fixed script.
    void taskId
    if (this.closed) return
    this.emitReplay()
    this.play(0)
  }

  close(): void {
    if (this.closed) return
    this.closed = true
    for (const timer of this.timers) clearTimeout(timer)
    this.timers.clear()
    this.setConnection("closed")
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

  private emitFrame(name: string, source: TaskFrame["source"]): void {
    for (const handler of this.frameHandlers) handler({ name, source })
  }

  private emitReplay(): void {
    const configured = this.scenario.replayPrefix ?? 0
    const forced = this.lateSubscribe
      ? Math.min(4, replayableStepCount(this.scenario))
      : 0
    const prefix = Math.min(
      this.scenario.steps.length,
      Math.max(configured, forced)
    )

    for (let index = 0; index < prefix; index += 1) {
      const step = this.scenario.steps[index]
      this.schedule(() => {
        if (!this.closed) this.emitFrame(step.event, "replay")
      }, 40 * index)
    }
  }

  private play(index: number): void {
    if (this.closed || index >= this.scenario.steps.length) return
    const step = this.scenario.steps[index]
    this.schedule(
      () => {
        if (this.closed) return
        this.emitFrame(step.event, "live")
        this.play(index + 1)
      },
      Math.max(0, step.delayMs / this.getSpeed())
    )
  }

  private schedule(callback: () => void, delayMs: number): void {
    const timer = setTimeout(() => {
      this.timers.delete(timer)
      callback()
    }, delayMs)
    this.timers.add(timer)
  }
}

function wait(delayMs: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, delayMs)
  })
}
