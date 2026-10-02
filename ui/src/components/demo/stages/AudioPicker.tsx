import {
  InfoCircle,
  MusicNote01,
  UploadCloud02,
} from "@untitledui/icons"
import { cn } from "cn"
import type { DragEvent } from "react"
import { useEffect, useRef, useState } from "react"

import { type AudioSample, getSampleAudio } from "@/api/sample-audio"
import { Button } from "@/components/ui/button"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { AudioInputController } from "@/hooks/useAudioInput"

interface AudioPickerProps {
  audio: AudioInputController
  disabled?: boolean
  onSelected?: (file: File) => void
  title?: string
  description?: string
}

const AUDIO_TYPES =
  "audio/mpeg,audio/wav,audio/x-wav,audio/mp4,audio/aac,audio/ogg,audio/webm"

export function AudioPicker({
  audio,
  disabled = false,
  onSelected,
  title = "Drop audio here",
  description = "MP3, WAV, M4A, AAC, OGG, WebM • 1 audio file",
}: AudioPickerProps) {
  const [dragging, setDragging] = useState(false)
  const [selectionError, setSelectionError] = useState<string | null>(null)
  const [loadingSample, setLoadingSample] = useState<AudioSample | null>(null)
  const sampleRequest = useRef<AbortController | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const selectionDisabled = disabled || loadingSample !== null

  useEffect(() => {
    if (disabled) sampleRequest.current?.abort()
    return () => sampleRequest.current?.abort()
  }, [disabled])

  const selectSample = async (sample: AudioSample) => {
    if (selectionDisabled || sampleRequest.current !== null) return
    const request = new AbortController()
    sampleRequest.current = request
    setLoadingSample(sample)
    setSelectionError(null)
    try {
      const file = await getSampleAudio(sample, request.signal)
      if (request.signal.aborted) return
      audio.selectFile(file)
      onSelected?.(file)
    } catch (error) {
      if (!request.signal.aborted) {
        setSelectionError(
          error instanceof Error ? error.message : "Unable to load sample audio"
        )
      }
    } finally {
      sampleRequest.current = null
      setLoadingSample(null)
    }
  }

  const selectFile = (file: File) => {
    if (selectionDisabled) return
    setSelectionError(null)
    audio.selectFile(file)
    onSelected?.(file)
  }

  const handleDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    setDragging(false)
    if (selectionDisabled) return
    const file = event.dataTransfer.files?.[0]
    if (file) selectFile(file)
  }

  return (
    <div className="mx-auto flex w-full max-w-xl flex-col gap-3">
      {/* biome-ignore lint/a11y/noStaticElementInteractions: HTML drag-and-drop has no semantic drop-zone element or ARIA role. */}
      <div
        onDragOver={(event) => {
          event.preventDefault()
          if (!selectionDisabled) setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={handleDrop}
        className={cn(
          "relative flex flex-col items-center rounded-[4px] border border-dashed bg-muted/35 text-center transition-colors",
          dragging ? "border-primary bg-primary/5" : "border-border",
          !selectionDisabled && "hover:border-primary/40",
          selectionDisabled && "cursor-not-allowed opacity-60"
        )}
      >
        <div className="flex min-h-60 w-full flex-col items-center justify-center px-4 py-8">
          <div className="mb-4 flex size-9 items-center justify-center rounded-lg bg-muted-foreground/20 text-muted-foreground">
            <MusicNote01 aria-hidden className="size-5" />
          </div>
          <p className="text-sm font-medium">{title}</p>
          <p className="mt-2 text-xs text-muted-foreground">{description}</p>
          <div className="mt-4 flex flex-wrap justify-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={selectionDisabled}
              onClick={() => inputRef.current?.click()}
            >
              <span className="relative top-px flex items-center gap-1">
                <UploadCloud02 aria-hidden />
                Choose audio
              </span>
            </Button>
          </div>
          <input
            ref={inputRef}
            type="file"
            accept={AUDIO_TYPES}
            className="sr-only"
            disabled={selectionDisabled}
            onChange={(event) => {
              const file = event.target.files?.[0]
              if (file) selectFile(file)
              event.currentTarget.value = ""
            }}
          />
        </div>
        <fieldset
          aria-label="Try a sample"
          className="flex w-full flex-col items-center justify-center gap-3 border-t border-border/60 px-4 py-4 sm:flex-row sm:gap-4"
        >
          <p className="text-xs text-muted-foreground">Or try a sample</p>
          <div className="flex flex-wrap justify-center gap-4">
            <button
              type="button"
              className="rounded-sm py-1 text-xs underline decoration-muted-foreground/60 decoration-dotted underline-offset-4 transition-colors hover:text-primary hover:decoration-primary focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-4 disabled:pointer-events-none disabled:opacity-50"
              disabled={selectionDisabled}
              onClick={() => void selectSample("neutral")}
            >
              {loadingSample === "neutral" ? "Loading neutral…" : "Neutral"}
            </button>
            <button
              type="button"
              className="rounded-sm py-1 text-xs underline decoration-muted-foreground/60 decoration-dotted underline-offset-4 transition-colors hover:text-primary hover:decoration-primary focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-4 disabled:pointer-events-none disabled:opacity-50"
              disabled={selectionDisabled}
              onClick={() => void selectSample("abusive")}
            >
              {loadingSample === "abusive"
                ? "Loading abusive chat…"
                : "Abusive chat"}
            </button>
          </div>
        </fieldset>
      </div>
      <div className="flex justify-end">
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              className="inline-flex items-center gap-1.5 rounded-sm py-1 text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-4"
            >
              <InfoCircle aria-hidden className="size-3.5" />
              Taking a while?
            </button>
          </TooltipTrigger>
          <TooltipContent sideOffset={6} className="text-center">
            This page may take longer than usual due to cold starts
          </TooltipContent>
        </Tooltip>
      </div>
      {selectionError !== null ? (
        <p className="text-center text-xs text-destructive" role="alert">
          {selectionError}
        </p>
      ) : null}
    </div>
  )
}
