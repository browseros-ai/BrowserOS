import { describe, expect, it } from 'bun:test'
import { getSubmittingPanelTabId, panelTabIdFromUrl } from './panel-host'

describe('submission panel ownership', () => {
  it('does not claim the active contextual tab from a window-wide panel', () => {
    const host = panelTabIdFromUrl('https://extension/sidepanel.html#/')
    expect(getSubmittingPanelTabId('sidepanel', host, 42)).toBeUndefined()
  })

  it('claims the native host even if another tab is active', () => {
    const host = panelTabIdFromUrl(
      'https://extension/sidepanel.html?tabId=17#/',
    )
    expect(getSubmittingPanelTabId('sidepanel', host, 42)).toBe(17)
  })

  it('allows a new-tab submitting document to join its conversation', () => {
    expect(getSubmittingPanelTabId('newtab', undefined, 17)).toBe(17)
  })
})
