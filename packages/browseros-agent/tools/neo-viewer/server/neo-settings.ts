/**
 * The settings the viewer gives a Neo it started itself (`PUT /v1/settings` on the host API):
 * Neo's trace keeps each call's arguments and result whole up to Neo's reply limit, so the
 * operation panel shows what the script's `await` got rather than a value cut at 2 KiB.
 *
 * Sent once the container is healthy, before the viewer serves, through `hostFetch`. Neo saves
 * it under the container's `/data` and loads it at every start, and the viewer sends it again at
 * each of its own starts. Never sent with `--upstream`: there the server is someone else's, and
 * the value would be saved in it and outlive the viewer.
 */

import { hostFetch } from './hostApi'

/** `trace.valueBytes`: Neo's reply limit, 256 KiB, the most Neo accepts. */
export const TRACE_VALUE_BYTES = 256 * 1024

/** The body of the viewer's `PUT`: everything else stays at Neo's defaults. */
export const VIEWER_SETTINGS = { trace: { valueBytes: TRACE_VALUE_BYTES } }

/** How long the viewer waits for the host API to answer after the container is healthy. */
const SETTINGS_WAIT_MS = 30_000
const SETTINGS_RETRY_MS = 500

export interface SettingsSeams {
  fetch?: typeof hostFetch
  sleep?: (ms: number) => Promise<void>
  waitMs?: number
}

/**
 * Sets the viewer's settings on its own Neo. `applied` once Neo answered with them;
 * `unsupported` from an image older than the setting (no `/v1/settings`, 404, or one that lists
 * `trace.valueBytes` in `unknown`), whose values stay cut at Neo's defaults. Throws when Neo
 * refused the body (a viewer bug) or the host API never answered.
 */
export async function applyViewerSettings(
  seams: SettingsSeams = {},
): Promise<'applied' | 'unsupported'> {
  const send = seams.fetch ?? hostFetch
  const sleep =
    seams.sleep ??
    ((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)))
  const waitMs = seams.waitMs ?? SETTINGS_WAIT_MS
  const deadline = Date.now() + waitMs
  for (;;) {
    // The whole wait is bounded, a request that never answers included: each attempt, its
    // answer's body too, gets what is left of it.
    const signal = AbortSignal.timeout(Math.max(deadline - Date.now(), 1))
    const answer = await attempt(send, signal).catch((error: unknown) => {
      if (signal.aborted) {
        throw new Error(
          `neo-viewer: Neo's host API did not answer the viewer's settings within ${waitMs} ms`,
        )
      }
      throw error
    })
    if (answer !== 'not serving') return answer
    if (Date.now() >= deadline) {
      throw new Error(
        `neo-viewer: Neo's host API was not serving within ${waitMs} ms, so the viewer's settings were not set`,
      )
    }
    await sleep(SETTINGS_RETRY_MS)
  }
}

/** One `PUT`: its outcome, or `not serving` while the host API cannot answer yet. */
async function attempt(
  send: typeof hostFetch,
  signal: AbortSignal,
): Promise<'applied' | 'unsupported' | 'not serving'> {
  const response = await send('/v1/settings', {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(VIEWER_SETTINGS),
    signal,
  })
  if (response.status === 404) return 'unsupported'
  // 502 and 503: the host API is not serving yet (Neo starts it once the browser links).
  if (response.status === 502 || response.status === 503) return 'not serving'
  if (!response.ok) {
    const problem = (await response.json().catch(() => ({}))) as {
      message?: string
    }
    throw new Error(
      `neo-viewer: Neo refused the viewer's settings (${response.status}): ${problem.message ?? 'no message'}`,
    )
  }
  const state = (await response.json()) as {
    settings?: { trace?: { valueBytes?: number } }
    unknown?: string[]
  }
  // An image with the settings but from before this one skips the name, as any unknown setting.
  if (state.unknown?.includes('trace.valueBytes')) return 'unsupported'
  if (state.settings?.trace?.valueBytes !== TRACE_VALUE_BYTES) {
    throw new Error(
      `neo-viewer: Neo answered /v1/settings without trace.valueBytes ${TRACE_VALUE_BYTES}`,
    )
  }
  return 'applied'
}
