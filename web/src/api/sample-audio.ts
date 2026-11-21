export type AudioSample = "neutral" | "abusive"

const SAMPLE_FILES: Record<AudioSample, string> = {
  neutral: "sample_071.mp3",
  abusive: "sample-1.mp3",
}

/** Load bundled demo audio lazily; selected samples use the normal upload flow. */
export async function getSampleAudio(
  sample: AudioSample,
  signal: AbortSignal
): Promise<File> {
  const name = SAMPLE_FILES[sample]
  const response = await fetch(`${import.meta.env.BASE_URL}samples/${name}`, {
    signal,
  })
  if (!response.ok)
    throw new Error("Unable to load sample audio. Please retry.")
  const blob = await response.blob()
  if (blob.size === 0 || !blob.type.startsWith("audio/")) {
    throw new Error("Sample audio is unavailable. Please retry.")
  }
  return new File([blob], name, { type: "audio/mpeg" })
}
