const STORAGE_KEY = "audio-moderation.evaluation-access.v1"

export interface EvaluationAccess {
  reviewId: string
  token: string
}

interface StoredAccess {
  version: 1
  evaluations: Record<string, EvaluationAccess>
}

interface StorageLike {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
}

function isAccess(value: unknown): value is EvaluationAccess {
  if (typeof value !== "object" || value === null) return false
  const access = value as Partial<EvaluationAccess>
  return typeof access.reviewId === "string" && typeof access.token === "string"
}

function parseAccess(
  serialized: string | null
): Record<string, EvaluationAccess> {
  if (serialized === null) return {}
  try {
    const value = JSON.parse(serialized) as Partial<StoredAccess>
    if (value.version !== 1 || typeof value.evaluations !== "object") return {}
    return Object.fromEntries(
      Object.entries(value.evaluations ?? {}).filter((entry) =>
        isAccess(entry[1])
      )
    )
  } catch {
    return {}
  }
}

function defaultStorage(): StorageLike | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.sessionStorage
  } catch {
    return undefined
  }
}

/** Keeps evaluation capabilities for the lifetime of the current browser tab. */
export class BrowserEvaluationAccessStore {
  private evaluations: Record<string, EvaluationAccess> = {}
  private storage: StorageLike | undefined

  constructor(storage: StorageLike | undefined = defaultStorage()) {
    this.storage = storage
    this.evaluations = this.read()
  }

  get(evaluationId: string): EvaluationAccess | undefined {
    const access = this.read()[evaluationId]
    return access === undefined ? undefined : { ...access }
  }

  set(evaluationId: string, access: EvaluationAccess): void {
    this.evaluations = { ...this.read(), [evaluationId]: { ...access } }
    this.write()
  }

  private read(): Record<string, EvaluationAccess> {
    if (this.storage === undefined) return this.evaluations
    try {
      this.evaluations = parseAccess(this.storage.getItem(STORAGE_KEY))
    } catch {
      this.storage = undefined
    }
    return this.evaluations
  }

  private write(): void {
    try {
      this.storage?.setItem(
        STORAGE_KEY,
        JSON.stringify({
          version: 1,
          evaluations: this.evaluations,
        } satisfies StoredAccess)
      )
    } catch {
      this.storage = undefined
    }
  }
}
