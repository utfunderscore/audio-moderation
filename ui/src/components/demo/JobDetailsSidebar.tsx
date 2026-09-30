import { cn } from "cn"
import {
  type CSSProperties,
  type PointerEvent,
  type ReactNode,
  type RefObject,
  useEffect,
  useRef,
  useState,
} from "react"

import { Button } from "@/components/ui/button"
import type { AudioProcessingJob } from "@/domain/jobs"
import { useMediaQuery } from "@/hooks/useMediaQuery"

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
  closing: boolean
  onClose: () => void
  onClosed: () => void
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
  closing,
  onClose,
  onClosed,
  children,
}: JobDetailsSidebarProps) {
  const separatorRef = useRef<HTMLDivElement>(null)
  const widthRef = useRef(DEFAULT_SIDEBAR_WIDTH)
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH)
  const [resizing, setResizing] = useState(false)
  const reducedMotion = useMediaQuery("(prefers-reduced-motion: reduce)")
  const [entered, setEntered] = useState(reducedMotion)
  const [moving, setMoving] = useState(!reducedMotion)
  const [previousClosing, setPreviousClosing] = useState(closing)

  // Pause content immediately when a close is requested or reversed, before
  // the browser starts the next shell transition.
  if (closing !== previousClosing) {
    setPreviousClosing(closing)
    setMoving(!reducedMotion)
  }

  useEffect(() => {
    const finishTransition = () => {
      if (closing) onClosed()
      else {
        setMoving(false)
        setEntered(true)
      }
    }
    if (reducedMotion) {
      finishTransition()
      return
    }

    // A breakpoint change can cancel the shell transition. Also settle when
    // the browser does not support starting-style and has nothing to animate.
    const frame = requestAnimationFrame(() => {
      const transitioning = panelRef.current
        ?.getAnimations()
        .some(
          (animation) =>
            animation instanceof CSSTransition &&
            animation.transitionProperty ===
              (isMobileViewport ? "transform" : "width") &&
            animation.playState === "running"
        )
      if (!transitioning) finishTransition()
    })
    return () => cancelAnimationFrame(frame)
  }, [closing, isMobileViewport, onClosed, panelRef, reducedMotion])

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
    // biome-ignore lint/a11y/useAriaPropsSupportedByRole: The panel is an aside on desktop and a modal dialog on mobile.
    <aside
      ref={panelRef}
      role={isMobileViewport ? "dialog" : undefined}
      aria-modal={isMobileViewport ? true : undefined}
      aria-label="Job details"
      data-state={closing ? "closing" : moving ? "opening" : "open"}
      data-resizing={resizing || undefined}
      onTransitionEnd={(event) => {
        if (event.target !== event.currentTarget) return
        const panelProperty = isMobileViewport ? "transform" : "width"
        if (event.propertyName !== panelProperty) return
        if (closing) onClosed()
        else {
          setMoving(false)
          setEntered(true)
        }
      }}
      style={
        {
          "--pipeline-width": `${sidebarWidth}px`,
        } as CSSProperties
      }
      className={cn(
        "pipeline-panel",
        "fixed inset-0 z-40 h-dvh shrink-0 overflow-hidden bg-background shadow-xl lg:sticky lg:inset-auto lg:top-14 lg:z-10 lg:h-[calc(100svh-3.5rem)] lg:shadow-none"
      )}
    >
      <div className="pipeline-panel-body relative flex h-full min-h-0 w-full flex-col border-l lg:w-[var(--pipeline-width)]">
        {/* biome-ignore lint/a11y/useSemanticElements: This is an interactive ARIA separator, which cannot use a semantic hr element. */}
        <div
          ref={separatorRef}
          role="separator"
          aria-label="Resize job details"
          aria-orientation="vertical"
          aria-valuemin={MIN_SIDEBAR_WIDTH}
          aria-valuemax={maximumSidebarWidth()}
          aria-valuenow={sidebarWidth}
          inert={!entered || closing}
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
          {entered ? children : null}
        </div>
      </div>
    </aside>
  )
}
