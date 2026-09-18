import { cn } from "cn"
import { Clock } from "@untitledui/icons"
import { useEffect, useState, type ReactNode } from "react"

import { Badge } from "@/components/ui/badge"
import { ElapsedDuration } from "@/components/demo/ElapsedDuration"
import {
  jobDisplayStatus,
  type AudioJobDisplayStatus,
  type AudioJobStatus,
  type AudioProcessingJob,
} from "@/domain/jobs"
import type { ModerationScores } from "@/domain/moderation"

interface Job {
  id: string
  fileName: string
  submitted: string
  duration: ReactNode
  status: AudioJobDisplayStatus
  current: boolean
  selected: boolean
  onOpen: () => void
}

const TIME_FORMATTER = new Intl.DateTimeFormat(undefined, {
  hour: "numeric",
  minute: "2-digit",
})

const DATE_FORMATTER = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
})

const DESKTOP_VIEWPORT_QUERY = "(min-width: 1024px)"

function useDesktopLayout() {
  const [desktop, setDesktop] = useState(
    () => window.matchMedia(DESKTOP_VIEWPORT_QUERY).matches
  )

  useEffect(() => {
    const mediaQuery = window.matchMedia(DESKTOP_VIEWPORT_QUERY)
    const updateLayout = () => setDesktop(mediaQuery.matches)
    mediaQuery.addEventListener("change", updateLayout)
    return () => mediaQuery.removeEventListener("change", updateLayout)
  }, [])

  return desktop
}

function formatSubmitted(timestamp: number) {
  const date = new Date(timestamp)
  const today = new Date()
  const yesterday = new Date()
  yesterday.setDate(today.getDate() - 1)

  const day = date.toDateString()
  const prefix =
    day === today.toDateString()
      ? "Today"
      : day === yesterday.toDateString()
        ? "Yesterday"
        : DATE_FORMATTER.format(date)

  return `${prefix}, ${TIME_FORMATTER.format(date)}`
}

function formatStoredDuration(milliseconds: number) {
  const seconds = Math.max(0, Math.round(milliseconds / 1_000))
  if (seconds < 60) return `${seconds}s`

  const minutes = Math.floor(seconds / 60)
  return `${minutes}m ${(seconds % 60).toString().padStart(2, "0")}s`
}

const STATUS_META: Record<
  AudioJobDisplayStatus,
  { label: string; className: string }
> = {
  processing: {
    label: "Processing",
    className: "border-warning/40 bg-warning/10 text-warning",
  },
  complete: {
    label: "Complete",
    className: "border-success/40 bg-success/10 text-success",
  },
  flagged: {
    label: "Flagged",
    className: "border-warning/60 bg-warning/15 text-warning",
  },
  failed: {
    label: "Failed",
    className: "border-destructive/40 bg-destructive/10 text-destructive",
  },
}

function StatusBadge({ status }: { status: AudioJobDisplayStatus }) {
  const meta = STATUS_META[status]
  return (
    <Badge variant="outline" className={cn("rounded-full", meta.className)}>
      {meta.label}
    </Badge>
  )
}

function JobRow({ job }: { job: Job }) {
  return (
    <tr
      tabIndex={0}
      aria-label={`Open job ${job.id}: ${job.fileName}`}
      aria-current={job.current ? "true" : undefined}
      className={cn(
        "group cursor-pointer transition-colors hover:bg-muted/40 focus-visible:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none focus-visible:ring-inset",
        job.selected && "bg-muted/50"
      )}
      onClick={job.onOpen}
      onKeyDown={(event) => {
        if (event.key !== "Enter" && event.key !== " ") return
        event.preventDefault()
        job.onOpen()
      }}
    >
      <td className="max-w-64 px-4 py-2.5">
        <div className="flex min-w-0 items-center gap-2 whitespace-nowrap">
          <span className="truncate font-medium">{job.fileName}</span>
          {job.current ? (
            <Badge variant="secondary" className="shrink-0 rounded-full">
              Current
            </Badge>
          ) : null}
        </div>
      </td>
      <td className="px-4 py-2.5 text-xs whitespace-nowrap text-muted-foreground">
        {job.submitted}
      </td>
      <td className="px-4 py-2.5 font-mono text-xs whitespace-nowrap text-muted-foreground">
        {job.duration}
      </td>
      <td className="px-4 py-2.5 text-xs whitespace-nowrap text-muted-foreground">
        #{job.id}
      </td>
      <td className="px-4 py-2.5">
        <StatusBadge status={job.status} />
      </td>
    </tr>
  )
}

function MobileJobItem({ job }: { job: Job }) {
  return (
    <button
      type="button"
      aria-label={`Open job ${job.id}: ${job.fileName}`}
      aria-current={job.current ? "true" : undefined}
      className={cn(
        "flex min-h-11 w-full cursor-pointer flex-col gap-3 border bg-card px-4 py-3 text-left shadow-sm transition-colors hover:bg-muted/40 focus-visible:relative focus-visible:z-10 focus-visible:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none focus-visible:ring-inset",
        job.selected && "bg-muted/50"
      )}
      onClick={job.onOpen}
    >
      <div className="flex w-full items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate font-medium">{job.fileName}</span>
            {job.current ? (
              <Badge variant="secondary" className="shrink-0 rounded-full">
                Current
              </Badge>
            ) : null}
          </div>
          <p className="mt-1 text-xs text-muted-foreground">{job.submitted}</p>
        </div>
        <StatusBadge status={job.status} />
      </div>

      <div className="grid w-full grid-cols-2 divide-x border-t pt-3 text-xs">
        <div className="min-w-0 pr-3">
          <p className="text-muted-foreground">Duration</p>
          <p className="mt-1 font-mono break-words">{job.duration}</p>
        </div>
        <div className="min-w-0 pl-3">
          <p className="text-muted-foreground">Job</p>
          <p className="mt-1 font-mono break-all">#{job.id}</p>
        </div>
      </div>
    </button>
  )
}

interface CurrentJob {
  id: string
  fileName: string
  startedAt: number
  endedAt?: number
  status: AudioJobStatus
  scores?: ModerationScores
}

export function JobHistory({
  currentJob,
  jobs,
  loading,
  error,
  selectedJobId,
  onOpenCurrent,
  onSelectJob,
}: {
  currentJob: CurrentJob | null
  jobs: readonly AudioProcessingJob[]
  loading: boolean
  error: string | null
  selectedJobId: string | null
  onOpenCurrent: () => void
  onSelectJob: (job: AudioProcessingJob) => void
}) {
  const desktop = useDesktopLayout()
  const current: Job | null =
    currentJob === null
      ? null
      : {
          id: currentJob.id,
          fileName: currentJob.fileName,
          submitted: formatSubmitted(currentJob.startedAt),
          duration: (
            <ElapsedDuration
              startedAt={currentJob.startedAt}
              endedAt={currentJob.endedAt}
              placeholder="Starting"
            />
          ),
          status: jobDisplayStatus(currentJob.status, currentJob.scores),
          current: true,
          selected: selectedJobId === null,
          onOpen: onOpenCurrent,
        }

  const previousJobs = jobs.filter((job) => job.id !== currentJob?.id)
  const displayJobs: Job[] = [
    ...(current === null ? [] : [current]),
    ...previousJobs.map((job) => ({
      id: job.id,
      fileName: job.fileName,
      submitted: formatSubmitted(job.submittedAt),
      duration:
        job.durationMs === null ? "—" : formatStoredDuration(job.durationMs),
      status: jobDisplayStatus(job.status, job.scores),
      current: false,
      selected: selectedJobId === job.id,
      onOpen: () => onSelectJob(job),
    })),
  ]
  const showLoading = loading && previousJobs.length === 0
  const showEmpty = !loading && error === null && displayJobs.length === 0

  return (
    <section aria-labelledby="jobs-heading">
      <div className="mb-4 flex items-end justify-between gap-4">
        <div>
          <h2 id="jobs-heading" className="text-lg font-semibold">
            Recent jobs
          </h2>
        </div>
        <div className="hidden items-center gap-1.5 text-xs text-muted-foreground sm:flex">
          <Clock aria-hidden className="size-3.5" />
          Updated just now
        </div>
      </div>

      {desktop ? (
        <div className="overflow-x-auto border bg-card shadow-sm">
          <table className="w-full min-w-[52rem] border-collapse text-left text-sm">
            <thead className="border-b bg-muted/35 text-xs text-muted-foreground">
              <tr>
                <th scope="col" className="px-4 py-2 font-medium">
                  Audio
                </th>
                <th scope="col" className="px-4 py-2 font-medium">
                  Submitted
                </th>
                <th scope="col" className="px-4 py-2 font-medium">
                  Duration
                </th>
                <th scope="col" className="px-4 py-2 font-medium">
                  Job
                </th>
                <th scope="col" className="px-4 py-2 font-medium">
                  Status
                </th>
              </tr>
            </thead>
            <tbody className="divide-y">
              {displayJobs.map((job) => (
                <JobRow key={job.id} job={job} />
              ))}
              {showLoading ? (
                <tr>
                  <td
                    colSpan={5}
                    className="px-4 py-6 text-center text-sm text-muted-foreground"
                  >
                    Loading jobs…
                  </td>
                </tr>
              ) : null}
              {error !== null ? (
                <tr>
                  <td
                    colSpan={5}
                    className="px-4 py-6 text-center text-sm text-destructive"
                  >
                    {error}
                  </td>
                </tr>
              ) : null}
              {showEmpty ? (
                <tr>
                  <td
                    colSpan={5}
                    className="px-4 py-6 text-center text-sm text-muted-foreground"
                  >
                    No recent jobs.
                  </td>
                </tr>
              ) : null}
            </tbody>
          </table>
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          {displayJobs.map((job) => (
            <MobileJobItem key={job.id} job={job} />
          ))}
          {showLoading ? (
            <p className="border bg-card px-4 py-6 text-center text-sm text-muted-foreground shadow-sm">
              Loading jobs…
            </p>
          ) : null}
          {error !== null ? (
            <p className="border bg-card px-4 py-6 text-center text-sm text-destructive shadow-sm">
              {error}
            </p>
          ) : null}
          {showEmpty ? (
            <p className="border bg-card px-4 py-6 text-center text-sm text-muted-foreground shadow-sm">
              No recent jobs.
            </p>
          ) : null}
        </div>
      )}
    </section>
  )
}
