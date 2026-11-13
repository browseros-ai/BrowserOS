import { afterAll, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { renderToStaticMarkup } from 'react-dom/server'
import * as auditHooks from '@/modules/api/audit.hooks'
import type { ReplaySession } from '@/screens/replay/use-replay-session'

mock.module('@/screens/replay/ReplayPanel', () => ({
  ReplayPanel: () => <div role="application" aria-label="Replay player" />,
}))

mock.module('@/modules/api/audit.hooks', () => ({
  ...auditHooks,
  useTaskScreenshotBaseUrl: () => 'http://127.0.0.1:9100',
}))

const { TaskPlayer } = await import('./TaskPlayer')

afterAll(() => mock.restore())

const recordedReplay = {
  plan: { durationSeconds: 42 },
  recordingState: 'settled',
} as unknown as ReplaySession

function renderPlayer(showReplay: boolean, snapshotId: number | null = 7) {
  const html = renderToStaticMarkup(
    <TaskPlayer
      sessionId="session-1"
      replay={recordedReplay}
      showReplay={showReplay}
      step={{ sentence: 'Opened the inbox' } as never}
      snapshotId={snapshotId}
      onFullScreen={() => undefined}
      fullScreen={false}
    />,
  )
  return parseHTML(`<!doctype html><html><body>${html}</body></html>`).document
}

describe('TaskPlayer', () => {
  it('tests that a recorded task shows its replay while replays are on', () => {
    const page = renderPlayer(true)
    expect(page.querySelector('[aria-label="Replay player"]')).not.toBeNull()
  })

  it('tests that turning replays off shows the step’s picture instead of the replay', () => {
    const page = renderPlayer(false)
    expect(page.querySelector('[aria-label="Replay player"]')).toBeNull()
    expect(page.querySelector('img')?.getAttribute('alt')).toBe(
      'What the page looked like: Opened the inbox',
    )
  })

  it('tests that turning replays off with no picture says there is no recording', () => {
    const page = renderPlayer(false, null)
    expect(page.querySelector('[aria-label="Replay player"]')).toBeNull()
    expect(page.body.textContent).toContain('No recording for this task')
  })
})
