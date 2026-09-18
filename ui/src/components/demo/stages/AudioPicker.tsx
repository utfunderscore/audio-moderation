import { Clipboard, MusicNote01, UploadCloud02 } from "@untitledui/icons"
import { cn } from "cn"
import type { DragEvent } from "react"
import { useRef, useState } from "react"

import { Button } from "@/components/ui/button"
import type { AudioInputController } from "@/hooks/useAudioInput"

interface AudioPickerProps {
  audio: AudioInputController
  disabled?: boolean
  onSelected?: () => void
  onSampleSelected?: (scenarioId: string) => void
  title?: string
  description?: string
}

const AUDIO_TYPES =
  "audio/mpeg,audio/wav,audio/x-wav,audio/mp4,audio/aac,audio/ogg,audio/webm"

const SAMPLE_SCENARIOS = [
  { id: "moderation-safe", label: "Safe conversation" },
  { id: "happy", label: "Personal info request" },
  { id: "moderation-harassment", label: "Harassment" },
] as const

export function AudioPicker({
  audio,
  disabled = false,
  onSelected,
  onSampleSelected,
  title = "Drop or paste audio here",
  description = "MP3, WAV, M4A, AAC, OGG, WebM • 1 audio file",
}: AudioPickerProps) {
  const [dragging, setDragging] = useState(false)
  const [clipboardError, setClipboardError] = useState<string | null>(null)
  const [loadingScenarioId, setLoadingScenarioId] = useState<string | null>(
    null
  )
  const inputRef = useRef<HTMLInputElement>(null)
  const selectionDisabled = disabled || audio.loadingSample

  const selectFile = (file: File) => {
    if (selectionDisabled) return
    setClipboardError(null)
    audio.selectFile(file)
    onSelected?.()
  }

  const handleDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    setDragging(false)
    if (selectionDisabled) return
    const file = event.dataTransfer.files?.[0]
    if (file) selectFile(file)
  }

  const pasteFromClipboard = async () => {
    if (selectionDisabled) return

    try {
      if (!navigator.clipboard?.read) {
        throw new Error("Clipboard audio is not supported in this browser")
      }

      const clipboardItems = await navigator.clipboard.read()
      for (const item of clipboardItems) {
        const type = item.types.find((clipboardType) =>
          clipboardType.startsWith("audio/")
        )
        if (type === undefined) continue

        const audioBlob = await item.getType(type)
        selectFile(new File([audioBlob], "pasted-audio", { type }))
        return
      }

      throw new Error("No audio file found on the clipboard")
    } catch (error) {
      setClipboardError(
        error instanceof Error ? error.message : "Unable to paste audio"
      )
    }
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
          "relative flex min-h-60 flex-col items-center justify-center rounded-[4px] border border-dashed bg-muted/35 px-4 pt-8 pb-16 text-center transition-colors",
          dragging ? "border-primary bg-primary/5" : "border-border",
          !selectionDisabled && "hover:border-primary/40",
          selectionDisabled && "cursor-not-allowed opacity-60"
        )}
      >
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
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={selectionDisabled}
            onClick={() => void pasteFromClipboard()}
          >
            <span className="relative top-px flex items-center gap-1">
              <Clipboard aria-hidden />
              Paste from clipboard
            </span>
          </Button>
        </div>
        <div className="absolute inset-x-4 bottom-4 text-center text-xs text-muted-foreground">
          <span>Or try an example: </span>
          {SAMPLE_SCENARIOS.map((scenario, index) => (
            <span key={scenario.id}>
              <button
                type="button"
                disabled={selectionDisabled}
                className="cursor-pointer border-b border-dotted border-current text-foreground outline-none hover:border-solid hover:text-primary focus-visible:border-solid focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
                onClick={() => {
                  setLoadingScenarioId(scenario.id)
                  void audio.loadSample().then((loaded) => {
                    setLoadingScenarioId(null)
                    if (!loaded) return
                    if (onSampleSelected !== undefined) {
                      onSampleSelected(scenario.id)
                    } else {
                      onSelected?.()
                    }
                  })
                }}
              >
                {scenario.label}
                {loadingScenarioId === scenario.id ? "…" : ""}
              </button>
              {index < SAMPLE_SCENARIOS.length - 1 ? ", " : ""}
            </span>
          ))}
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
      {clipboardError !== null ? (
        <p className="text-center text-xs text-destructive">{clipboardError}</p>
      ) : null}
      {audio.error !== null ? (
        <p className="text-center text-xs text-destructive">{audio.error}</p>
      ) : null}
    </div>
  )
}
