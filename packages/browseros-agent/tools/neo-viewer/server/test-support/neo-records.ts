/** Neo's records of runs for tests, built from Neo's own wire shape (openapi.yaml `Run`). */

import type { NeoRunRecord, RunDetail } from '../../shared/api'
import { textSize } from '../../shared/api'
import { runDetail, runSummary } from '../../web/test-support/runs'
import { toNeoRunRecord } from '../neo-runs'

/**
 * A two-line script's record: `page.goto` on line 1, `locator.click` on line 2 (`clickFails`:
 * it failed with `strict_mode_violation`), and Neo's own `events.listen` and a nested
 * `frame.waitForLoadState`, which are on no line.
 */
export function twoLineRecord(clickFails = false): NeoRunRecord {
  return toNeoRunRecord({
    id: '01a1',
    status: clickFails ? 'error' : 'ok',
    effects: 'done',
    startedAt: 1_000,
    endedAt: 1_900,
    complete: true,
    tabs: [7],
    error: clickFails
      ? {
          owner: 'script',
          code: 'strict_mode_violation',
          message: 'locator.click: strict mode violation: 2 elements',
          retryable: false,
        }
      : null,
    operations: [
      {
        id: '01a1.1',
        name: 'events.listen',
        kind: 'builtin',
        effect: 'read',
        startedAt: 1_000,
        durationMs: 2,
        outcome: 'ok',
      },
      {
        id: '01a1.2',
        name: 'page.goto',
        kind: 'builtin',
        effect: 'write',
        site: { line: 1, column: 7 },
        tabId: 7,
        args: { url: 'https://example.com' },
        result: null,
        startedAt: 1_010,
        durationMs: 412,
        outcome: 'ok',
        delivery: 'done',
      },
      {
        id: '01a1.3',
        parentId: '01a1.2',
        name: 'frame.waitForLoadState',
        kind: 'builtin',
        effect: 'read',
        startedAt: 1_050,
        durationMs: 200,
        outcome: 'ok',
      },
      {
        id: '01a1.4',
        name: 'locator.click',
        kind: 'builtin',
        effect: 'write',
        site: { line: 2, column: 7 },
        tabId: 7,
        args: { selector: 'a' },
        startedAt: 1_425,
        durationMs: 80,
        outcome: clickFails ? 'error' : 'ok',
        delivery: clickFails ? 'not_sent' : 'done',
        ...(clickFails
          ? {
              error: {
                owner: 'script',
                code: 'strict_mode_violation',
                message: 'locator.click: strict mode violation: 2 elements',
                retryable: false,
              },
            }
          : {}),
      },
    ],
  })
}

export const TWO_LINES =
  "await page.goto('https://example.com')\nawait page.click('a')"

/** A finished run's detail with Neo's record (`neo: null` for none) and the text the model read. */
export function detailWith(
  seq: number,
  options: {
    code?: string
    text: string
    neo: NeoRunRecord | null
    isError?: boolean
    agentTaskId?: string | null
  },
): RunDetail {
  const code = options.code ?? TWO_LINES
  const summary = runSummary(seq, {
    code,
    reply: options.text,
    status: options.isError ? 'error' : 'ok',
    roundTripMs: 1_000,
  })
  const detail = runDetail(summary, options.text)
  return {
    ...detail,
    agentTaskId: options.agentTaskId ?? null,
    neo: options.neo,
    neoDurationMs: options.neo?.durationMs ?? null,
    operationCount: options.neo ? options.neo.operations.length : null,
    replySize: textSize(options.text),
    reply: { ...detail.reply, isError: options.isError ?? false },
  }
}
