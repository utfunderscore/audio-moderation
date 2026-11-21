import { ApiBackend } from "@/api/api-backend"

/**
 * Composition root. Construct the real transport once; pass `backend` to
 * `useEvaluation` and `useJobHistory`. Browser-only APIs (storage, Turnstile)
 * are accessed lazily, so importing this during SSR is safe.
 */
export const backend = new ApiBackend()
