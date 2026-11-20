/**
 * The scrape door: `GET /v1/metrics` on Neo's host API (bearer token added by `hostFetch`), every
 * 2 s while a host API is configured. Silent by design — a dev tool must not fill the terminal
 * because the image is old or still starting:
 * - 200: the text goes to `onText`;
 * - 404: an image older than telemetry; the poller stops for the life of the process;
 * - anything else (401, the container still starting, a slow answer): try again in 10 s.
 */

export interface PollerDeps {
  /** A host API request (`hostFetch`), path and query only. */
  fetch: (path: string, init?: RequestInit) => Promise<Response>
  onText: (text: string, at: number) => void
  now: () => number
  intervalMs?: number
  retryMs?: number
}

const INTERVAL_MS = 2_000
const RETRY_MS = 10_000
/** A scrape that takes longer than this is abandoned and tried again later. */
const TIMEOUT_MS = 5_000
/** Neo's metrics are a few kilobytes; a body past this is not read. */
const MAX_TEXT_BYTES = 4 * 1024 * 1024

export interface Poller {
  /** 'absent' once Neo answered 404: this image has no metrics. */
  state(): 'polling' | 'absent' | 'stopped'
  stop(): Promise<void>
}

export function pollMetrics(deps: PollerDeps): Poller {
  const stopping = new AbortController()
  let state: 'polling' | 'absent' | 'stopped' = 'polling'
  const loop = (async () => {
    while (!stopping.signal.aborted) {
      const delay = await scrapeOnce(deps, stopping.signal)
      if (delay === null) {
        state = 'absent'
        return
      }
      await sleep(delay, stopping.signal)
    }
  })()
  return {
    state: () => state,
    async stop() {
      if (state === 'polling') state = 'stopped'
      stopping.abort()
      await loop
    },
  }
}

/** One scrape; the wait before the next, or null when Neo has no metrics route. */
async function scrapeOnce(
  deps: PollerDeps,
  stopping: AbortSignal,
): Promise<number | null> {
  const interval = deps.intervalMs ?? INTERVAL_MS
  const retry = deps.retryMs ?? RETRY_MS
  try {
    const response = await deps.fetch('/v1/metrics', {
      signal: AbortSignal.any([stopping, AbortSignal.timeout(TIMEOUT_MS)]),
    })
    if (response.status === 404) {
      await response.body?.cancel()
      return null
    }
    if (!response.ok) {
      await response.body?.cancel()
      return retry
    }
    const text = await readText(response)
    if (text === null) return retry
    deps.onText(text, deps.now())
    return interval
  } catch {
    return retry
  }
}

/** The body as text, or null past `MAX_TEXT_BYTES`. */
async function readText(response: Response): Promise<string | null> {
  if (Number(response.headers.get('content-length') ?? 0) > MAX_TEXT_BYTES) {
    await response.body?.cancel()
    return null
  }
  const chunks: Uint8Array[] = []
  let size = 0
  for await (const chunk of response.body ?? []) {
    size += chunk.byteLength
    if (size > MAX_TEXT_BYTES) return null
    chunks.push(chunk)
  }
  return new TextDecoder().decode(Buffer.concat(chunks))
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted) return resolve()
    const timer = setTimeout(done, ms)
    function done() {
      clearTimeout(timer)
      signal.removeEventListener('abort', done)
      resolve()
    }
    signal.addEventListener('abort', done, { once: true })
  })
}
