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
 * `unsupported` from an image older than `/v1/settings` (404), whose values stay cut at Neo's
 * defaults. Throws when Neo refused the body (a viewer bug) or the host API never answered.
 */
export async function applyViewerSettings(
  seams: SettingsSeams = {},
): Promise<'applied' | 'unsupported'> {
  const send = seams.fetch ?? hostFetch
  const sleep =
    seams.sleep ??
    ((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)))
  const deadline = Date.now() + (seams.waitMs ?? SETTINGS_WAIT_MS)
  for (;;) {
    const response = await send('/v1/settings', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(VIEWER_SETTINGS),
    })
    if (response.status === 404) return 'unsupported'
    if (response.ok) {
      const state = (await response.json()) as {
        settings?: { trace?: { valueBytes?: number } }
      }
      if (state.settings?.trace?.valueBytes !== TRACE_VALUE_BYTES) {
        throw new Error(
          `neo-viewer: Neo answered /v1/settings without trace.valueBytes ${TRACE_VALUE_BYTES}`,
        )
      }
      return 'applied'
    }
    // 502 and 503: the host API is not serving yet (Neo starts it once the browser links).
    const waiting = response.status === 502 || response.status === 503
    if (!waiting || Date.now() >= deadline) {
      const problem = (await response.json().catch(() => ({}))) as {
        message?: string
      }
      throw new Error(
        `neo-viewer: Neo refused the viewer's settings (${response.status}): ${problem.message ?? 'no message'}`,
      )
    }
    await sleep(SETTINGS_RETRY_MS)
  }
}
