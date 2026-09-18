let sampleBlobPromise: Promise<Blob> | null = null

export function preloadDemoAudioArtifact(): Promise<Blob> {
  sampleBlobPromise ??= fetch(`${import.meta.env.BASE_URL}audio/sample_071.mp3`)
    .then((response) => {
      if (!response.ok) throw new Error("audio artifact unavailable")
      return response.blob()
    })
    .catch((error: unknown) => {
      sampleBlobPromise = null
      throw error
    })
  return sampleBlobPromise
}
