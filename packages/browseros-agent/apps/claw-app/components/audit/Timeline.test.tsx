/**
 * Timeline display and interaction checks for script grouping, code previews,
 * and expansion state across live audit updates.
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import { TabView } from '@/screens/task-detail/TabView'
import { groupDispatchesByTab } from '@/screens/task-detail/task-detail.helpers'
import { Timeline } from './Timeline'

function dispatch(overrides: Partial<ToolDispatchRow> = {}): ToolDispatchRow {
  return {
    dispatchId: 1,
    createdAt: 1_000_000,
    slug: 'a',
    label: 'A',
    sessionId: 'sess',
    toolName: 'snapshot',
    pageId: 1,
    argsJson: '{"page":1}',
    resultMeta: '{"isError":false}',
    durationMs: 5,
    ...overrides,
  }
}

const startedAt = 1_000_000

function render(
  dispatches: ToolDispatchRow[],
  extra: { showSessionEnd?: boolean; endEvent?: TimelineEndEvent } = {},
): string {
  return renderToStaticMarkup(
    <Timeline
      dispatches={dispatches}
      startedAt={startedAt}
      endEvent={extra.endEvent ?? null}
      showSessionEnd={extra.showSessionEnd}
      onScreenshotClick={() => undefined}
    />,
  )
}

type TimelineEndEvent = {
  createdAt: number
  kind: 'closed' | 'errored' | 'cancelled'
  reason: string | null
} | null

const CLOSED_END: TimelineEndEvent = {
  createdAt: startedAt + 60_000,
  kind: 'closed',
  reason: null,
}

const CANCELLED_END: TimelineEndEvent = {
  createdAt: startedAt + 30_000,
  kind: 'cancelled',
  reason: 'operator requested stop',
}

describe('Timeline', () => {
  it.each(['{"code":7}', 'null', '[]', '{"code":"truncated'])(
    'keeps non-script or malformed args as JSON: %s',
    (argsJson) => {
      const html = render([dispatch({ toolName: 'act', argsJson })])
      expect(html).toContain('aria-label="Copy args"')
      expect(html).not.toContain('data-language="javascript"')
    },
  )

  it('renders empty script code with a copy control', () => {
    const html = render([
      dispatch({ toolName: 'custom-script', argsJson: '{"code":""}' }),
    ])
    expect(html).toContain('data-language="javascript"')
    expect(html).toContain('aria-label="Copy code"')
  })

  it('keeps orphan children at the top level without inventing a parent', () => {
    const html = render([
      dispatch({ parentDispatchId: 'missing', toolName: 'page.goto' }),
    ])
    const { document } = parseHTML(html)
    expect(document.querySelector('ol > li > button')?.textContent).toContain(
      'page.goto',
    )
    expect(document.querySelector('ol ol')).toBeNull()
    expect(html).not.toContain('steps')
  })

  it('preserves the script name in a page tab without rendering its page-less parent', () => {
    const rows = [
      dispatch({
        dispatchId: 1,
        dispatchKey: 'script',
        toolName: 'playwright',
        pageId: undefined,
      }),
      dispatch({
        dispatchId: 2,
        parentDispatchId: 'script',
        toolName: 'page.goto',
        pageId: 7,
      }),
    ]
    const group = groupDispatchesByTab(rows, []).find(
      (candidate) => candidate.pageId === 7,
    )
    if (!group) throw new Error('Missing page group')
    const html = renderToStaticMarkup(
      <TabView
        sessionId="sess"
        group={group}
        allDispatches={rows}
        startedAt={startedAt}
        endEvent={null}
        onScreenshotClick={() => undefined}
      />,
    )
    const { document } = parseHTML(html)
    expect(document.querySelector('ol > li > button')?.textContent).toContain(
      'in playwright script',
    )
    expect(html).toContain('page.goto')
    expect(html).not.toContain('timeline-block-copy-args')
    expect(html).not.toContain('Still running')
  })

  it('auto-expands any code-bearing tool and shows bounded JavaScript with a copy button', () => {
    const code = Array.from(
      { length: 45 },
      (_, i) => `const value${i + 1} = ${i + 1};`,
    ).join('\n')
    const html = render([
      dispatch({
        toolName: 'custom-script',
        argsJson: JSON.stringify({ code }),
      }),
    ])
    const { document } = parseHTML(html)
    const block = document.querySelector('[data-language="javascript"]')
    expect(block?.querySelector('pre code')?.textContent).toContain(
      'const value1 = 1;',
    )
    expect(block?.textContent).toContain('const value40 = 40;')
    expect(block?.textContent).not.toContain('const value41')
    expect(html).toContain('Show all 45 lines')
    expect(html).toContain('aria-label="Copy code"')
    expect(html).not.toContain('&quot;code&quot;')
  })

  it('shows failed child steps in the parent header even when the parent succeeded', () => {
    const html = render([
      dispatch({ dispatchId: 1, dispatchKey: 'script', toolName: 'run' }),
      dispatch({
        dispatchId: 2,
        parentDispatchId: 'script',
        resultMeta: '{"isError":true}',
      }),
    ])
    const { document } = parseHTML(html)
    const header = document.querySelector('ol > li > button')
    expect(header?.textContent).toContain('1 step')
    expect(header?.textContent).toContain('Step failed')
    const childHeader = document.querySelector('ol ol > li > button')
    expect(childHeader?.textContent).toContain('Failed')
    expect(childHeader?.getAttribute('aria-expanded')).toBe('false')
  })

  it('nests script steps in timestamp and dispatch-id order under an auto-expanded parent', () => {
    const html = render([
      dispatch({
        dispatchId: 9,
        parentDispatchId: 'script',
        toolName: 'locator.click',
        createdAt: startedAt + 2,
      }),
      dispatch({
        dispatchId: 8,
        parentDispatchId: 'script',
        toolName: 'locator.fill',
        createdAt: startedAt + 2,
      }),
      dispatch({
        dispatchId: 7,
        parentDispatchId: 'script',
        toolName: 'page.goto',
        createdAt: startedAt + 1,
      }),
      dispatch({
        dispatchId: 10,
        dispatchKey: 'script',
        toolName: 'custom-script',
        pageId: undefined,
      }),
    ])
    const { document } = parseHTML(html)
    const parent = document.querySelector('ol > li')
    expect(parent?.textContent).toContain('custom-script')
    expect(parent?.textContent).toContain('3 steps')
    const steps = [...(parent?.querySelectorAll('ol > li') ?? [])]
    expect(steps).toHaveLength(3)
    expect(steps[0]?.textContent).toContain('page.goto')
    expect(steps[1]?.textContent).toContain('locator.fill')
    expect(steps[2]?.textContent).toContain('locator.click')
    expect(
      steps[0]?.querySelector('button')?.getAttribute('aria-expanded'),
    ).toBe('false')
  })

  it('renders Expand all + Collapse all buttons', () => {
    const html = render([
      dispatch({ dispatchId: 1 }),
      dispatch({ dispatchId: 2 }),
    ])
    expect(html).toContain('Expand all')
    expect(html).toContain('Collapse all')
  })

  it('disables Collapse all on first render when nothing is auto-expanded', () => {
    const html = render([dispatch({ dispatchId: 1, toolName: 'snapshot' })])
    // Collapse all should be disabled (no notable rows auto-expanded).
    expect(html).toMatch(
      /<button[^>]*data-disabled=""[^>]*timeline-collapse-all/,
    )
  })

  it('leaves Collapse all enabled when a notable row auto-expands', () => {
    const html = render([
      dispatch({ dispatchId: 1, toolName: 'snapshot' }),
      dispatch({ dispatchId: 2, toolName: 'act' }),
    ])
    expect(html).not.toMatch(
      /<button[^>]*data-disabled=""[^>]*timeline-collapse-all/,
    )
  })

  it('auto-expands notable rows so args + result blocks appear on initial render', () => {
    const html = render([
      dispatch({
        dispatchId: 7,
        toolName: 'act',
        argsJson: '{"kind":"click","ref":"btn-submit"}',
        resultMeta: '{"isError":false,"structuredKeys":["clicked"]}',
      }),
    ])
    // JSON in HTML markup is rendered with HTML-entity quotes; check
    // for the inner tokens that survive entity encoding instead.
    expect(html).toContain('btn-submit')
    expect(html).toContain('structuredKeys')
    expect(html).toContain('clicked')
  })

  it('disables both buttons when the dispatch list is empty', () => {
    const html = render([])
    expect(html).toMatch(/<button[^>]*data-disabled=""[^>]*timeline-expand-all/)
    expect(html).toMatch(
      /<button[^>]*data-disabled=""[^>]*timeline-collapse-all/,
    )
  })

  it('renders a copy button per block (args, result, page) on an expanded row', () => {
    const html = render([
      dispatch({
        dispatchId: 1,
        toolName: 'act',
        argsJson: '{"kind":"click"}',
        resultMeta: '{"isError":false}',
        url: 'https://example.com',
      }),
    ])
    expect(html).toContain('data-testid="timeline-block-copy-args"')
    expect(html).toContain('data-testid="timeline-block-copy-result"')
    expect(html).toContain('data-testid="timeline-block-copy-page"')
  })

  it('does not render a copy button on the screenshot block (image, not text)', () => {
    const html = render([
      dispatch({
        dispatchId: 2,
        toolName: 'screenshot',
        screenshotId: 9,
        argsJson: '{"page":1}',
        resultMeta: '{"isError":false}',
      }),
    ])
    expect(html).not.toContain('data-testid="timeline-block-copy-screenshot"')
  })

  it('renders a captured screenshot for an errored dispatch', () => {
    const html = render([
      dispatch({
        dispatchId: 3,
        toolName: 'act',
        screenshotId: 10,
        resultMeta: '{"isError":true}',
      }),
    ])
    expect(html).toContain('>screenshot<')
  })

  it('renders the SessionEndRow by default', () => {
    const html = render([dispatch({ dispatchId: 1 })], {
      endEvent: CLOSED_END,
    })
    // SessionEndRow markup includes the literal "session closed" (or
    // "session errored") tail; either is enough to pin the presence
    // of the row.
    expect(html).toContain('session closed')
  })

  it('renders an operator-stopped terminal row', () => {
    const html = render([dispatch()], { endEvent: CANCELLED_END })
    expect(html).toContain('session stopped')
  })

  it('hides the SessionEndRow when showSessionEnd is false (per-tab view)', () => {
    const html = render([dispatch({ dispatchId: 1 })], {
      endEvent: CLOSED_END,
      showSessionEnd: false,
    })
    expect(html).not.toContain('session closed')
    expect(html).not.toContain('session errored')
    expect(html).not.toContain('Still running')
    // The dispatch row itself is still there.
    expect(html).toContain('snapshot')
  })
})

describe('Timeline interactions', () => {
  const globalNames = [
    'window',
    'document',
    'navigator',
    'HTMLElement',
    'Node',
    'Event',
    'location',
    'IS_REACT_ACT_ENVIRONMENT',
  ]
  let descriptors: Map<string, PropertyDescriptor | undefined>
  let root: Root
  let container: HTMLElement
  let copied: string[]

  beforeEach(async () => {
    descriptors = new Map(
      globalNames.map((name) => [
        name,
        Object.getOwnPropertyDescriptor(globalThis, name),
      ]),
    )
    const dom = parseHTML(
      '<!doctype html><html><body><div id="root"></div></body></html>',
    )
    Object.assign(dom.window, {
      location: { search: '?apiUrl=http://127.0.0.1:9210' },
    })
    copied = []
    const globals = {
      window: dom.window,
      document: dom.document,
      navigator: {
        clipboard: {
          writeText: async (text: string) => {
            copied.push(text)
          },
        },
      },
      HTMLElement: dom.window.HTMLElement,
      Node: dom.window.Node,
      Event: dom.window.Event,
      IS_REACT_ACT_ENVIRONMENT: true,
    }
    for (const [name, value] of Object.entries(globals)) {
      Object.defineProperty(globalThis, name, {
        configurable: true,
        writable: true,
        value,
      })
    }
    container = dom.document.getElementById('root') as unknown as HTMLElement
    const { createRoot } = await import('react-dom/client')
    root = createRoot(container)
  })

  afterEach(async () => {
    await act(async () => root.unmount())
    for (const [name, descriptor] of descriptors) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor)
      else Reflect.deleteProperty(globalThis, name)
    }
  })

  async function mount(dispatches: ToolDispatchRow[]) {
    await act(async () =>
      root.render(
        <Timeline
          dispatches={dispatches}
          startedAt={startedAt}
          endEvent={null}
          showSessionEnd={false}
          onScreenshotClick={() => undefined}
        />,
      ),
    )
  }

  async function click(selector: string) {
    const button = container.querySelector<HTMLButtonElement>(selector)
    if (!button) throw new Error(`Missing button: ${selector}`)
    await act(async () => button.click())
  }

  it('keeps child expansion independent of the parent and supports expand/collapse all', async () => {
    await mount([
      dispatch({
        dispatchId: 1,
        dispatchKey: 'script',
        toolName: 'playwright',
      }),
      dispatch({
        dispatchId: 2,
        parentDispatchId: 'script',
        toolName: 'locator.click',
        argsJson: '{"selector":"submit-button"}',
      }),
    ])
    expect(container.querySelector('ol ol pre')).toBeNull()
    await click('ol ol > li > button')
    expect(container.querySelector('ol ol pre')?.textContent).toContain(
      'submit-button',
    )
    await click('ol > li > button')
    expect(container.querySelector('ol ol')).toBeNull()
    expect(container.textContent).toContain('1 step')
    await click('ol > li > button')
    expect(container.querySelector('ol ol pre')?.textContent).toContain(
      'submit-button',
    )
    await click('[data-testid="timeline-collapse-all"]')
    expect(container.querySelector('ol ol')).toBeNull()
    await click('[data-testid="timeline-expand-all"]')
    expect(container.querySelector('ol ol pre')?.textContent).toContain(
      'submit-button',
    )
  })

  it('regroups late parents and preserves user toggles across polls', async () => {
    const child = dispatch({
      dispatchId: 2,
      parentDispatchId: 'script',
      toolName: 'page.goto',
    })
    const parent = dispatch({
      dispatchId: 1,
      dispatchKey: 'script',
      toolName: 'custom-script',
    })
    await mount([child])
    expect(container.querySelectorAll('ol > li')).toHaveLength(1)
    await mount([child, parent])
    expect(container.querySelector('ol ol')?.textContent).toContain('page.goto')
    await click('ol > li > button')
    await mount([
      child,
      parent,
      dispatch({ dispatchId: 3, parentDispatchId: 'script' }),
    ])
    expect(container.textContent).toContain('2 steps')
    expect(container.querySelector('ol ol')).toBeNull()
  })

  it('expands long code, copies the full script from the preview, and highlights JavaScript', async () => {
    const code = Array.from(
      { length: 42 },
      (_, i) => `console.log(${i + 1});`,
    ).join('\n')
    await mount([
      dispatch({ toolName: 'run', argsJson: JSON.stringify({ code }) }),
    ])
    expect(container.querySelector('pre code')?.textContent).not.toContain(
      'console.log(42)',
    )
    await click('[aria-label="Copy code"]')
    expect(copied).toEqual([code])
    const toggle = () =>
      [...container.querySelectorAll('button')].find((button) =>
        button.textContent?.includes('Show '),
      )
    await act(async () => toggle()?.click())
    expect(container.querySelector('pre code')?.textContent).toContain(
      'console.log(42)',
    )
    await act(async () => toggle()?.click())
    expect(container.querySelector('pre code')?.textContent).not.toContain(
      'console.log(42)',
    )
    // The real async highlighter eventually replaces the immediately readable
    // raw <pre> with themed tokens; no highlighter mock can mask a broken path.
    for (
      let attempt = 0;
      attempt < 100 && !container.innerHTML.includes('--shiki-dark:');
      attempt++
    ) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 10))
      })
    }
    expect(container.innerHTML).toContain('--shiki-dark:')
  })
})
