import { cn } from "cn"
import { useEffect, useRef, useState } from "react"

const BUCKETS = 140

interface WaveformProps {
  file: File
  /** 0..1 playback position. */
  progress: number
}

/**
 * Decodes the selected file and draws a compact peak waveform. Decoding is
 * best-effort: an unsupported codec falls back to a static placeholder.
 */
export function Waveform({ file, progress }: WaveformProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const [peaks, setPeaks] = useState<number[] | null>(null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    let cancelled = false

    const context = new AudioContext()
    file
      .arrayBuffer()
      .then((data) => context.decodeAudioData(data))
      .then((audioBuffer) => {
        if (cancelled) return
        const channel = audioBuffer.getChannelData(0)
        const bucketSize = Math.max(1, Math.floor(channel.length / BUCKETS))
        const next: number[] = []
        for (let index = 0; index < BUCKETS; index += 1) {
          const start = index * bucketSize
          let sum = 0
          for (let offset = 0; offset < bucketSize; offset += 1) {
            const value = channel[start + offset] ?? 0
            sum += value * value
          }
          next.push(Math.sqrt(sum / bucketSize))
        }
        const max = Math.max(...next, 0.0001)
        setPeaks(next.map((peak) => peak / max))
      })
      .catch(() => {
        if (!cancelled) setFailed(true)
      })
      .finally(() => {
        void context.close()
      })

    return () => {
      cancelled = true
      void context.close()
    }
  }, [file])

  useEffect(() => {
    const canvas = canvasRef.current
    if (canvas === null) return

    const draw = () => {
      const width = canvas.clientWidth
      const height = canvas.clientHeight
      if (width === 0 || height === 0) return

      const ratio = window.devicePixelRatio || 1
      canvas.width = Math.floor(width * ratio)
      canvas.height = Math.floor(height * ratio)
      const context = canvas.getContext("2d")
      if (context === null) return

      context.setTransform(ratio, 0, 0, ratio, 0, 0)
      context.clearRect(0, 0, width, height)

      const values = peaks ?? Array.from({ length: BUCKETS }, () => 0.22)
      const color = getComputedStyle(canvas).color
      const barWidth = width / values.length
      const playedIndex = Math.round(progress * values.length)

      for (let index = 0; index < values.length; index += 1) {
        const amplitude = failed ? 0.25 : values[index]
        const barHeight = Math.max(2, amplitude * height * 0.92)
        const x = index * barWidth
        const y = (height - barHeight) / 2
        context.globalAlpha = index < playedIndex ? 1 : 0.3
        context.fillStyle = color
        context.fillRect(
          x + barWidth * 0.15,
          y,
          Math.max(1, barWidth * 0.7),
          barHeight
        )
      }
      context.globalAlpha = 1
    }

    draw()
    const observer = new ResizeObserver(draw)
    observer.observe(canvas)
    return () => observer.disconnect()
  }, [peaks, progress, failed])

  return (
    <div className={cn("relative")}>
      <canvas
        ref={canvasRef}
        className="h-16 w-full text-primary"
        aria-hidden
      />
      {failed ? (
        <p className="absolute inset-0 flex items-center justify-center text-xs text-muted-foreground">
          Waveform unavailable for this file
        </p>
      ) : null}
    </div>
  )
}
