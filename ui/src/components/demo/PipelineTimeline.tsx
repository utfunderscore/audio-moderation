import type { ReactNode } from "react"

/** Vertical timeline container for the stage pages. */
export function PipelineTimeline({ children }: { children: ReactNode }) {
  return (
    <ol className="m-0 flex list-none flex-col gap-8 p-0">{children}</ol>
  )
}
