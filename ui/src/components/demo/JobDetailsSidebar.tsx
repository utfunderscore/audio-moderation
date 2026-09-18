import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent,
  type ReactNode,
  type RefObject,
} from "react"
import { cn } from "cn"

import { Button } from "@/components/ui/button"
import type { AudioProcessingJob } from "@/domain/jobs"

const DEFAULT_SIDEBAR_WIDTH = 448
const MIN_SIDEBAR_WIDTH = 360
const MAX_SIDEBAR_WIDTH = 720
const MIN_MAIN_WIDTH = 480

function maximumSidebarWidth() {
  return Math.max(
    MIN_SIDEBAR_WIDTH,
    Math.min(MAX_SIDEBAR_WIDTH, window.innerWidth - MIN_MAIN_WIDTH)
  )
}

function clampSidebarWidth(width: number) {
  return Math.min(maximumSidebarWidth(), Math.max(MIN_SIDEBAR_WIDTH, width))
}

interface JobDetailsSidebarProps {
  panelRef: RefObject<HTMLElement | null>
  contentRef: RefObject<HTMLDivElement | null>
  closeButtonRef: RefObject<HTMLButtonElement | null>
  selectedJob: AudioProcessingJob | null
  audioFileName?: string
  isMobileViewport: boolean
  onClose: () => void
  children: ReactNode
}

/** Contains resizing so pointer moves update only this panel's DOM. */
export function JobDetailsSidebar({
  panelRef,
  contentRef,
  closeButtonRef,
  selectedJob,
  audioFileName,
  isMobileViewport,
  onClose,
  children,
}: JobDetailsSidebarProps) {
  const separatorRef = useRef<HTMLDivElement>(null)
  const widthRef = useRef(DEFAULT_SIDEBAR_WIDTH)
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH)
  const [resizing, setResizing] = useState(false)

  const applySidebarWidth = (width: number) => {
    widthRef.current = width
    panelRef.current?.style.setProperty("--pipeline-width", `${width}px`)
    separatorRef.current?.setAttribute("aria-valuenow", String(width))
  }

  const commitSidebarWidth = (width: number) => {
    applySidebarWidth(width)
    setSidebarWidth(width)
  }

  useEffect(() => {
    if (!resizing) return undefined

    const previousCursor = document.body.style.cursor
    const previousUserSelect = document.body.style.userSelect
    document.body.style.cursor = "col-resize"
    document.body.style.userSelect = "none"

    return () => {
      document.body.style.cursor = previousCursor
      document.body.style.userSelect = previousUserSelect
    }
  }, [resizing])

  useEffect(() => {
    const updateMaximumWidth = () => {
      const maximumWidth = maximumSidebarWidth()
      separatorRef.current?.setAttribute("aria-valuemax", String(maximumWidth))
      if (widthRef.current > maximumWidth) {
        widthRef.current = maximumWidth
        panelRef.current?.style.setProperty(
          "--pipeline-width",
          `${maximumWidth}px`
        )
        separatorRef.current?.setAttribute(
          "aria-valuenow",
          String(maximumWidth)
        )
        setSidebarWidth(maximumWidth)
      }
    }

    window.addEventListener("resize", updateMaximumWidth)
    return () => window.removeEventListener("resize", updateMaximumWidth)
  }, [panelRef])

  const stopResizing = (event: PointerEvent<HTMLDivElement>) => {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
    setSidebarWidth(widthRef.current)
    setResizing(false)
  }

  return (
    <aside
      ref={panelRef}
      role={isMobileViewport ? "dialog" : undefined}
      aria-modal={isMobileViewport ? true : undefined}
      aria-label="Job details"
      style={
        {
          "--pipeline-width": `${sidebarWidth}px`,
        } as CSSProperties
      }
      className={cn(
        "pipeline-panel-enter",
        "fixed inset-0 z-40 flex h-dvh w-full shrink-0 flex-col overflow-hidden border-l bg-background shadow-xl lg:sticky lg:inset-auto lg:top-14 lg:z-10 lg:h-[calc(100svh-3.5rem)] lg:w-[var(--pipeline-width)] lg:overflow-visible lg:shadow-none",
        !resizing && "transition-[width] duration-200"
      )}
    >
      <div
        ref={separatorRef}
        role="separator"
        aria-label="Resize job details"
        aria-orientation="vertical"
        aria-valuemin={MIN_SIDEBAR_WIDTH}
        aria-valuemax={maximumSidebarWidth()}
        aria-valuenow={sidebarWidth}
        tabIndex={0}
        title="Drag to resize. Double-click to reset."
        className="group absolute inset-y-0 -left-2 z-30 hidden w-4 cursor-col-resize touch-none items-center justify-center outline-none lg:flex"
        onDoubleClick={() =>
          commitSidebarWidth(clampSidebarWidth(DEFAULT_SIDEBAR_WIDTH))
        }
        onKeyDown={(event) => {
          let nextWidth: number | null = null

          if (event.key === "ArrowLeft") nextWidth = widthRef.current + 24
          if (event.key === "ArrowRight") nextWidth = widthRef.current - 24
          if (event.key === "Home") nextWidth = MIN_SIDEBAR_WIDTH
          if (event.key === "End") nextWidth = maximumSidebarWidth()

          if (nextWidth === null) return
          event.preventDefault()
          commitSidebarWidth(clampSidebarWidth(nextWidth))
        }}
        onPointerDown={(event) => {
          event.preventDefault()
          event.currentTarget.setPointerCapture(event.pointerId)
          setResizing(true)
        }}
        onPointerMove={(event) => {
          if (!event.currentTarget.hasPointerCapture(event.pointerId)) return
          applySidebarWidth(
            clampSidebarWidth(window.innerWidth - event.clientX)
          )
        }}
        onPointerUp={stopResizing}
        onPointerCancel={stopResizing}
      >
        <span
          aria-hidden
          className={cn(
            "h-full w-px bg-transparent transition-colors group-hover:bg-ring group-focus-visible:bg-ring",
            resizing && "bg-ring"
          )}
        />
        <span
          aria-hidden
          className={cn(
            "absolute h-10 w-1 bg-border transition-colors group-hover:bg-ring group-focus-visible:bg-ring",
            resizing && "bg-ring"
          )}
        />
      </div>
      <div className="flex h-14 shrink-0 items-center justify-between border-b px-4">
        <div className="min-w-0">
          <h2 className="truncate text-sm font-semibold">
            {selectedJob === null ? "Current job" : `Job #${selectedJob.id}`}
          </h2>
          <p className="truncate text-xs text-muted-foreground">
            {selectedJob?.fileName ?? audioFileName ?? "No job selected"}
          </p>
        </div>
        <Button
          ref={closeButtonRef}
          type="button"
          variant="ghost"
          size="icon"
          aria-label="Close job details"
          onClick={onClose}
        >
          <span aria-hidden className="text-lg leading-none">
            ×
          </span>
        </Button>
      </div>

      <div
        ref={contentRef}
        className="min-h-0 flex-1 touch-pan-y overflow-y-auto overscroll-contain p-5 pb-[calc(1.25rem+env(safe-area-inset-bottom))] lg:overscroll-auto"
      >
        {children}
      </div>
    </aside>
  )
}
