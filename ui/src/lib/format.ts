export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

/** Wall-clock timestamp with milliseconds, e.g. 12:00:01.204. */
export function formatClock(timestamp: number): string {
  const date = new Date(timestamp)
  const pad = (value: number, length = 2) =>
    value.toString().padStart(length, "0")
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(
    date.getSeconds()
  )}.${pad(date.getMilliseconds(), 3)}`
}

/** Media position, e.g. 1:04. */
export function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00"
  const whole = Math.floor(seconds)
  const minutes = Math.floor(whole / 60)
  const remainder = whole % 60
  return `${minutes}:${remainder.toString().padStart(2, "0")}`
}
