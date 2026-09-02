import { beforeEach, describe, expect, it, mock } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

let dismissed = false
mock.module('@/lib/cloud-sync/cloud-sync-storage', () => ({
  cloudSyncNoticeDismissedStorage: {
    getValue: async () => dismissed,
    setValue: async (value: boolean) => {
      dismissed = value
    },
  },
}))

const { CloudSyncRetiredNotice } = await import('./CloudSyncRetiredNotice')

beforeEach(() => {
  dismissed = false
})

describe('CloudSyncRetiredNotice', () => {
  // Dismissal is read asynchronously from extension storage, so the first
  // paint must not flash a banner the user already dismissed.
  it('renders nothing before the dismissal state is known', () => {
    const html = renderToStaticMarkup(createElement(CloudSyncRetiredNotice))
    expect(html).toBe('')
  })
})

describe('the copy', () => {
  const source = require('node:fs').readFileSync(
    new URL('./CloudSyncRetiredNotice.tsx', import.meta.url).pathname,
    'utf8',
  )

  // Sync stops in the same release this ships, so a future-tense warning
  // would describe something that has already happened.
  it('states what changed rather than warning about it', () => {
    expect(source).toContain('has been turned off')
    expect(source).not.toMatch(/will (stop|soon)/i)
  })

  // The question people actually have is whether they are losing anything.
  it('says what keeps working and what does not', () => {
    expect(source).toContain('keep working')
    expect(source).toContain('history')
  })
})
