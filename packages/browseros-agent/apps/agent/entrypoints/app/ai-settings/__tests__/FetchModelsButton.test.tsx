/**
 * TDD — FetchModelsButton edge cases & gotchas
 *
 * Covers regressions from replacing <Button> with custom styled <button>:
 *   G1. Missing focus-visible ring (regression from Button component)
 *   G2. Missing aria-label (accessibility)
 *   G3. Error state — no error element before fetch attempted
 *   G4. Disabled state styling presence
 *   G5. Idle state shows correct icon + text
 *   G6. Successful fetch calls onFetchComplete with model IDs
 *   G7. Failed fetch does NOT call onFetchComplete
 *   G8. Button type="button" prevents form submission
 *   G9. Icon SVG renders in idle state
 *
 * Run: bun test entrypoints/app/ai-settings/__tests__/FetchModelsButton.test.tsx
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'bun:test'
import { createElement, type ReactNode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { Window } from 'happy-dom'

// ─── Set up happy-dom globals ───────────────────────────────────────────────

const happyWindow = new Window()
globalThis.document = happyWindow.document as unknown as Document
globalThis.window = happyWindow as unknown as Window & typeof globalThis
globalThis.navigator = happyWindow.navigator as unknown as Navigator
const happyGlobal = happyWindow as unknown as Record<string, unknown>
if (!happyGlobal.SyntaxError) happyGlobal.SyntaxError = SyntaxError
if (!happyGlobal.TypeError) happyGlobal.TypeError = TypeError
if (!happyGlobal.Error) happyGlobal.Error = Error
if (!happyGlobal.RangeError) happyGlobal.RangeError = RangeError

// ─── Mocks (must be before dynamic imports) ─────────────────────────────────

const mockFetchModelsFromApi = vi.fn()

vi.mock('@/lib/utils', () => ({
  cn: (...args: unknown[]) =>
    args.filter((a) => typeof a === 'string' && a.length > 0).join(' '),
}))

vi.mock('@/lib/llm-providers/fetchModels', () => ({
  fetchModelsFromApi: (...args: unknown[]) => mockFetchModelsFromApi(...args),
}))

vi.mock('lucide-react', () => ({
  CloudDownload: (props: Record<string, unknown>) =>
    createElement('svg', { 'data-testid': 'icon-cloud-download', ...props }),
  Loader2: (props: Record<string, unknown>) =>
    createElement('svg', { 'data-testid': 'icon-loader', ...props }),
}))

// ─── Dynamic import AFTER mocks ─────────────────────────────────────────────

const { FetchModelsButton } = await import('../FetchModelsButton')

// ─── DOM helpers ───────────────────────────────────────────────────────────

let container: HTMLDivElement
let root: Root

async function renderToDOM(element: ReactNode): Promise<HTMLDivElement> {
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
  root.render(element)
  // Flush React async render
  await new Promise((r) => setTimeout(r, 10))
  return container
}

function cleanupDOM() {
  try { root?.unmount() } catch {}
  if (container?.parentNode) container.parentNode.removeChild(container)
}

/** Query for first element matching tag inside container */
function $(el: HTMLDivElement, tag: string): HTMLElement | null {
  const found = el.getElementsByTagName(tag)
  return found.length > 0 ? (found[0] as HTMLElement) : null
}

// ─── Mock data ─────────────────────────────────────────────────────────────

const successResult = {
  success: true as const,
  models: [
    { id: 'gpt-4o', contextLength: 128000, source: 'fetched' as const },
    { id: 'gpt-4o-mini', contextLength: 128000, source: 'fetched' as const },
  ],
}

const errorResult = {
  success: false as const,
  models: [] as Array<{ id: string; contextLength: number; source: 'fetched' }>,
  error: 'HTTP 401',
}

const BASE = 'https://api.openai.com/v1'
const KEY = 'sk-test-key'

// ─── Lifecycle ─────────────────────────────────────────────────────────────

beforeEach(() => { vi.clearAllMocks() })
afterEach(() => { cleanupDOM() })

// ═══════════════════════════════════════════════════════════════════════════
// G1. Focus-visible ring (REGRESSION from removing <Button>)
// ═══════════════════════════════════════════════════════════════════════════

describe('G1. focus-visible ring', () => {
  it('button has focus-visible styles in className', async () => {
    mockFetchModelsFromApi.mockResolvedValue(successResult)
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete: () => {} }),
    )
    const btn = $(el, 'button')!
    expect(btn).toBeTruthy()
    // REGRESSION: old <Button variant="outline"> had focus-visible ring.
    expect(btn.className).toMatch(/focus-visible/)
  })

  it('button has outline-none in className', async () => {
    mockFetchModelsFromApi.mockResolvedValue(successResult)
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete: () => {} }),
    )
    const btn = $(el, 'button')!
    expect(btn.className).toContain('outline-none')
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// G2. Accessibility — aria-label
// ═══════════════════════════════════════════════════════════════════════════

describe('G2. accessibility', () => {
  it('button has aria-label attribute', async () => {
    mockFetchModelsFromApi.mockResolvedValue(successResult)
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete: () => {} }),
    )
    const btn = $(el, 'button')!
    expect(btn.getAttribute('aria-label')).toBeTruthy()
  })

  it('aria-label describes the action (contains fetch/download/model)', async () => {
    mockFetchModelsFromApi.mockResolvedValue(successResult)
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete: () => {} }),
    )
    const btn = $(el, 'button')!
    const label = btn.getAttribute('aria-label')?.toLowerCase() ?? ''
    expect(label).toMatch(/fetch|download|model/)
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// G3. Error state
// ═══════════════════════════════════════════════════════════════════════════

describe('G3. error state', () => {
  it('does NOT show error element before fetch is attempted', async () => {
    mockFetchModelsFromApi.mockResolvedValue(successResult)
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete: () => {} }),
    )
    expect($(el, 'p')).toBeNull()
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// G4. Disabled state styling
// ═══════════════════════════════════════════════════════════════════════════

describe('G4. disabled state', () => {
  it('button is disabled when disabled prop is true', async () => {
    mockFetchModelsFromApi.mockResolvedValue(successResult)
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete: () => {}, disabled: true }),
    )
    const btn = $(el, 'button')!
    expect(btn.disabled).toBe(true)
  })

  it('button has disabled:opacity style', async () => {
    mockFetchModelsFromApi.mockResolvedValue(successResult)
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete: () => {} }),
    )
    expect($(el, 'button')!.className).toMatch(/disabled:opacity/)
  })

  it('button has disabled:pointer-events-none', async () => {
    mockFetchModelsFromApi.mockResolvedValue(successResult)
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete: () => {} }),
    )
    expect($(el, 'button')!.className).toMatch(/disabled:pointer-events-none/)
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// G5. Idle state
// ═══════════════════════════════════════════════════════════════════════════

describe('G5. idle state', () => {
  it('shows "Fetch from API" text', async () => {
    mockFetchModelsFromApi.mockResolvedValue(successResult)
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete: () => {} }),
    )
    expect($(el, 'button')!.textContent).toContain('Fetch from API')
  })

  it('renders cloud-download icon (not loader)', async () => {
    mockFetchModelsFromApi.mockResolvedValue(successResult)
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete: () => {} }),
    )
    const svgs = el.getElementsByTagName('svg')
    expect(svgs.length).toBeGreaterThan(0)
    let foundCloudDownload = false
    for (const svg of svgs) {
      if (svg.getAttribute('data-testid') === 'icon-cloud-download') foundCloudDownload = true
    }
    expect(foundCloudDownload).toBe(true)
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// G8. Button type attribute
// ═══════════════════════════════════════════════════════════════════════════

describe('G8. button type', () => {
  it('has type="button" to prevent form submission', async () => {
    mockFetchModelsFromApi.mockResolvedValue(successResult)
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete: () => {} }),
    )
    expect($(el, 'button')!.getAttribute('type')).toBe('button')
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// G9. Icon SVG
// ═══════════════════════════════════════════════════════════════════════════

describe('G9. icon rendering', () => {
  it('renders an SVG icon element', async () => {
    mockFetchModelsFromApi.mockResolvedValue(successResult)
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete: () => {} }),
    )
    expect(el.getElementsByTagName('svg').length).toBeGreaterThan(0)
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// G6 & G7. Callback contracts
// ═══════════════════════════════════════════════════════════════════════════

describe('G6. successful fetch callback', () => {
  it('calls onFetchComplete with model IDs', async () => {
    mockFetchModelsFromApi.mockResolvedValue(successResult)
    const onFetchComplete = vi.fn()
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete }),
    )
    $(el, 'button')!.click()
    // Wait for async fetch + React re-render
    for (let i = 0; i < 50; i++) {
      await new Promise((r) => setTimeout(r, 5))
      if (onFetchComplete.mock.calls.length > 0) break
    }
    expect(onFetchComplete).toHaveBeenCalledTimes(1)
    expect(onFetchComplete).toHaveBeenCalledWith(['gpt-4o', 'gpt-4o-mini'])
  })

  it('does NOT call onFetchComplete after failed fetch', async () => {
    mockFetchModelsFromApi.mockResolvedValue(errorResult)
    const onFetchComplete = vi.fn()
    const el = await renderToDOM(
      createElement(FetchModelsButton, { baseUrl: BASE, apiKey: KEY, onFetchComplete }),
    )
    $(el, 'button')!.click()
    // Wait for async fetch + React re-render
    for (let i = 0; i < 50; i++) {
      await new Promise((r) => setTimeout(r, 5))
      if (mockFetchModelsFromApi.mock.calls.length > 0) break
    }
    expect(onFetchComplete).not.toHaveBeenCalled()
  })
})
