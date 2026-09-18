/* eslint-disable react-hooks/immutability -- playback exclusively mutates the
   DOM media element from event handlers. */
import { MusicNote01, Play, Trash01 } from "@untitledui/icons"
import { useEffect, useState } from "react"

import { Button } from "@/components/ui/button"
import { Slider } from "@/components/ui/slider"
import type { AudioInputController } from "@/hooks/useAudioInput"
import { formatBytes, formatTime } from "@/lib/format"
import { Waveform } from "../Waveform"

interface AudioPlayerProps {
  audio: AudioInputController
  disabled?: boolean
  onRemove?: () => void
  removable?: boolean
}

export function AudioPlayer({
  audio,
  disabled = false,
  onRemove,
  removable = true,
}: AudioPlayerProps) {
  return (
    <AudioPlayerControls
      key={audio.url}
      audio={audio}
      disabled={disabled}
      onRemove={onRemove}
      removable={removable}
    />
  )
}

function AudioPlayerControls({
  audio,
  disabled = false,
  onRemove,
  removable = true,
}: AudioPlayerProps) {
  const [playing, setPlaying] = useState(() => !audio.audioRef.current?.paused)
  const [currentTime, setCurrentTime] = useState(
    () => audio.audioRef.current?.currentTime ?? 0
  )
  const [duration, setDuration] = useState(
    () => audio.audioRef.current?.duration || 0
  )

  useEffect(() => {
    const element = audio.audioRef.current
    if (element === null) return undefined

    const updateTime = () => setCurrentTime(element.currentTime)
    const updateDuration = () => setDuration(element.duration || 0)
    const markPlaying = () => setPlaying(true)
    const markPaused = () => setPlaying(false)

    element.addEventListener("timeupdate", updateTime)
    element.addEventListener("loadedmetadata", updateDuration)
    element.addEventListener("play", markPlaying)
    element.addEventListener("pause", markPaused)
    element.addEventListener("ended", markPaused)

    return () => {
      element.removeEventListener("timeupdate", updateTime)
      element.removeEventListener("loadedmetadata", updateDuration)
      element.removeEventListener("play", markPlaying)
      element.removeEventListener("pause", markPaused)
      element.removeEventListener("ended", markPaused)
    }
  }, [audio.audioRef, audio.url])

  const toggle = () => {
    const element = audio.audioRef.current
    if (element === null) return
    if (element.paused) {
      void element.play()
    } else {
      element.pause()
    }
  }

  const seek = (seconds: number) => {
    const element = audio.audioRef.current
    if (element === null) return
    element.currentTime = seconds
    setCurrentTime(seconds)
  }

  const progress = duration > 0 ? Math.min(1, currentTime / duration) : 0

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-start justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <MusicNote01
            aria-hidden
            className="size-4 shrink-0 text-muted-foreground"
          />
          <div className="min-w-0">
            <p className="truncate text-sm font-medium">{audio.file?.name}</p>
            <p className="text-xs text-muted-foreground">
              {audio.file === null ? "" : formatBytes(audio.file.size)}
            </p>
          </div>
        </div>
        {removable ? (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Remove audio"
            disabled={disabled}
            onClick={onRemove ?? audio.clear}
          >
            <Trash01 />
          </Button>
        ) : null}
      </div>

      {audio.file !== null ? (
        <Waveform
          key={`${audio.file.name}-${audio.file.size}-${audio.file.lastModified}`}
          file={audio.file}
          progress={progress}
        />
      ) : null}

      <div className="flex items-center gap-3">
        <Button
          variant="outline"
          size="icon-sm"
          aria-label={playing ? "Pause" : "Play"}
          onClick={toggle}
        >
          {playing ? (
            <span aria-hidden className="flex h-3 items-stretch gap-1">
              <span className="w-0.5 bg-current" />
              <span className="w-0.5 bg-current" />
            </span>
          ) : (
            <Play />
          )}
        </Button>
        <Slider
          value={[currentTime]}
          min={0}
          max={Math.max(duration, 0.001)}
          step={0.01}
          onValueChange={(value) => seek(value[0] ?? 0)}
          aria-label="Seek"
          className="flex-1"
        />
        <span className="w-20 shrink-0 text-right font-mono text-xs text-muted-foreground">
          {formatTime(currentTime)} / {formatTime(duration)}
        </span>
      </div>
    </div>
  )
}
