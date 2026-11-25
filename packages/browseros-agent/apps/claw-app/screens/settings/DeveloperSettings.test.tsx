import { describe, expect, it } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { usePlanDevTools } from '@/modules/api/plan.hooks'
import { DeveloperSettings } from './DeveloperSettings'
import { forgetPlanMemory } from './developer-settings.helpers'

function render(developmentBuild: boolean, offered?: boolean): string {
  const client = new QueryClient()
  if (offered !== undefined) {
    client.setQueryData(usePlanDevTools.getKey(), offered)
  }
  const html = renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <DeveloperSettings developmentBuild={developmentBuild} />
    </QueryClientProvider>,
  )
  client.clear()
  return html
}

class MemoryStorage implements Storage {
  private items = new Map<string, string>()
  get length() {
    return this.items.size
  }
  clear() {
    this.items.clear()
  }
  getItem(key: string) {
    return this.items.get(key) ?? null
  }
  key(index: number) {
    return [...this.items.keys()][index] ?? null
  }
  removeItem(key: string) {
    this.items.delete(key)
  }
  setItem(key: string, value: string) {
    this.items.set(key, value)
  }
}

describe('Developer settings', () => {
  it('offers removing the licence on a debug server', () => {
    const html = render(true, true)
    expect(html).toContain('Developer')
    expect(html).toContain('Remove licence from this computer')
    expect(html).toContain('Debug build')
    expect(html).toContain(
      'This computer goes back to Free and gets a new install id, so you can test the free trial again.',
    )
  })

  it('shows nothing where the server has no developer tools, or while it checks', () => {
    expect(render(true, false)).toBe('')
    expect(render(true)).toBe('')
  })

  it('shows nothing in a release build of the app', () => {
    expect(render(false)).toBe('')
  })

  it('forgets only what the app remembers about the plan', () => {
    const storage = new MemoryStorage()
    for (const key of [
      'proBannerDismissed:v1',
      'ultrafastTrialBarDismissed:v1',
      'ultrafastTrialEndingBarDismissed:v1',
      'ultrafastTrialWatch:v1',
      'skillUpgradeSeen:v1:skills:reorder:1',
      'productHuntBannerDismissed',
      'theme',
    ]) {
      storage.setItem(key, 'true')
    }
    forgetPlanMemory(storage)
    expect(
      [...Array(storage.length).keys()].map((i) => storage.key(i)),
    ).toEqual(['productHuntBannerDismissed', 'theme'])
  })
})
