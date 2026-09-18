import { AlertCircle, Clock, Loading01, SlashCircle01 } from "@untitledui/icons"
import { cn } from "cn"
import type { ReactNode } from "react"

import { Badge } from "@/components/ui/badge"
import { formatDuration } from "@/domain/reducer"
import type { StageState } from "@/domain/stages"

const NODE_CLASS: Record<StageState, string> = {
  pending: "border-white/20 bg-white/5 text-white",
  processing: "border-white/20 bg-white/10 text-white",
  complete: "border-white/20 bg-white/10 text-white",
  failed: "border-white/20 bg-white/10 text-white",
  skipped: "border-white/10 bg-white/5 text-white/60",
}

const BADGE_CLASS: Record<StageState, string> = {
  pending: "text-muted-foreground",
  processing: "border-warning/40 bg-warning/10 text-warning",
  complete: "border-success/40 bg-success/10 text-success",
  failed: "border-destructive/40 bg-destructive/10 text-destructive",
  skipped: "text-muted-foreground/70",
}

const STATE_LABEL: Record<StageState, string> = {
  pending: "Pending",
  processing: "Processing",
  complete: "Complete",
  failed: "Failed",
  skipped: "Skipped",
}

const STATE_ICON: Record<StageState, ReactNode> = {
  pending: <Clock aria-hidden />,
  processing: (
    <Loading01
      aria-hidden
      className="animate-spin motion-reduce:animate-none"
    />
  ),
  // The label already says "Complete"; a check glyph adds nothing.
  complete: null,
  failed: <AlertCircle aria-hidden />,
  skipped: <SlashCircle01 aria-hidden />,
}

interface StagePageProps {
  title: string
  subtitle?: ReactNode
  icon: ReactNode
  status: StageState
  elapsed: number | null
  elapsedContent?: ReactNode
  isLast?: boolean
  children: ReactNode
}

/** One page of the pipeline timeline: node, title, status, and its artifact. */
export function StagePage({
  title,
  subtitle,
  icon,
  status,
  elapsed,
  elapsedContent,
  isLast = false,
  children,
}: StagePageProps) {
  return (
    <li
      className="pipeline-stage-enter relative pl-0 lg:pl-12"
      data-status={status}
    >
      <span
        aria-hidden
        className={cn(
          "pipeline-stage-node absolute top-0 left-0 hidden size-8 items-center justify-center rounded-full border bg-card transition-colors duration-500 lg:flex [&>svg]:size-4",
          NODE_CLASS[status]
        )}
      >
        {icon}
      </span>
      {!isLast ? (
        <span
          aria-hidden
          className="pipeline-connector absolute top-9 -bottom-8 left-4 hidden w-px origin-top bg-border lg:block"
        />
      ) : null}

      <div className="flex flex-col gap-3">
        <div className="flex min-w-0 items-center justify-between gap-3">
          <div className="flex min-w-0 items-center gap-3">
            <span
              aria-hidden
              className={cn(
                "pipeline-stage-node flex size-8 shrink-0 items-center justify-center rounded-full border bg-card transition-colors duration-500 lg:hidden [&>svg]:size-4",
                NODE_CLASS[status]
              )}
            >
              {icon}
            </span>
            <div className="flex min-w-0 flex-col">
              <h2 className="truncate text-sm font-medium">{title}</h2>
              {subtitle !== undefined ? (
                <p className="truncate text-xs text-muted-foreground">
                  {subtitle}
                </p>
              ) : null}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {elapsed !== null || elapsedContent !== undefined ? (
              <span className="pipeline-status-enter font-mono text-xs text-muted-foreground">
                {elapsedContent ?? formatDuration(elapsed)}
              </span>
            ) : null}
            {status !== "complete" ? (
              <Badge
                key={status}
                variant="outline"
                className={cn(
                  "pipeline-status-enter gap-1 transition-colors duration-300",
                  BADGE_CLASS[status]
                )}
              >
                {STATE_ICON[status]}
                {STATE_LABEL[status]}
              </Badge>
            ) : null}
          </div>
        </div>

        <div key={status} className="pipeline-content-enter min-w-0">
          {children}
        </div>
      </div>
    </li>
  )
}
