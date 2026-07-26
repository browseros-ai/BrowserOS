import { describe, expect, test } from 'bun:test'
import type { AXNode } from './ax-types'
import { RefMap } from './refs'
import { renderSnapshot } from './render'

function ax(
  nodeId: string,
  role: string,
  opts: Partial<AXNode> & { children?: string[] } = {},
): AXNode {
  const { children, ...rest } = opts
  return {
    nodeId,
    role: { type: 'role', value: role },
    childIds: children,
    ...rest,
  }
}

function name(value: string): AXNode['name'] {
  return { type: 'computedString', value }
}

describe('renderSnapshot', () => {
  test('renders semantic tree with refs only on actionable nodes', () => {
    const nodes: AXNode[] = [
      ax('1', 'RootWebArea', { children: ['2', '6'] }),
      ax('2', 'navigation', { children: ['3'] }),
      ax('3', 'link', { name: name('Home'), backendDOMNodeId: 101 }),
      ax('6', 'main', { children: ['7', '8', '9'] }),
      ax('7', 'heading', {
        name: name('Results'),
        backendDOMNodeId: 110,
        properties: [{ name: 'level', value: { type: 'integer', value: 1 } }],
      }),
      ax('8', 'button', {
        name: name('Load more'),
        backendDOMNodeId: 111,
        properties: [
          { name: 'disabled', value: { type: 'boolean', value: true } },
        ],
      }),
      ax('9', 'generic', { children: ['10'] }),
      ax('10', 'textbox', {
        name: name('Search'),
        backendDOMNodeId: 112,
        value: { type: 'string', value: 'abc' },
      }),
    ]

    const out = renderSnapshot(nodes, { refs: new RefMap() })

    expect(out.text).toBe(
      [
        '- navigation',
        '  - link "Home" [ref=e1]',
        '- main',
        '  - heading "Results" [level=1]',
        '  - button "Load more" [disabled] [ref=e2]',
        '  - textbox "Search" [ref=e3]: "abc"',
      ].join('\n'),
    )
  })

  test('drops unnamed generic wrappers, lifting children to parent depth', () => {
    const nodes: AXNode[] = [
      ax('1', 'RootWebArea', { children: ['2'] }),
      ax('2', 'generic', { children: ['3'] }),
      ax('3', 'button', { name: name('Go'), backendDOMNodeId: 5 }),
    ]
    expect(renderSnapshot(nodes, { refs: new RefMap() }).text).toBe(
      '- button "Go" [ref=e1]',
    )
  })

  test('mints distinct nth for duplicate role+name within a frame', () => {
    const refs = new RefMap()
    const nodes: AXNode[] = [
      ax('1', 'RootWebArea', { children: ['2', '3'] }),
      ax('2', 'button', { name: name('OK'), backendDOMNodeId: 1 }),
      ax('3', 'button', { name: name('OK'), backendDOMNodeId: 2 }),
    ]
    renderSnapshot(nodes, { refs })

    expect(refs.get('e1')).toMatchObject({
      backendNodeId: 1,
      nth: 0,
      name: 'OK',
    })
    expect(refs.get('e2')).toMatchObject({
      backendNodeId: 2,
      nth: 1,
      name: 'OK',
    })
  })

  test('reuses refs for same-document backend nodes after insertion', () => {
    const refs = new RefMap()
    const before: AXNode[] = [
      ax('1', 'RootWebArea', { children: ['2', '3'] }),
      ax('2', 'button', { name: name('A'), backendDOMNodeId: 1 }),
      ax('3', 'link', { name: name('B'), backendDOMNodeId: 2 }),
    ]
    const after: AXNode[] = [
      ax('1', 'RootWebArea', { children: ['4', '2', '3'] }),
      ax('4', 'button', { name: name('X'), backendDOMNodeId: 3 }),
      ax('2', 'button', { name: name('A'), backendDOMNodeId: 1 }),
      ax('3', 'link', { name: name('B'), backendDOMNodeId: 2 }),
    ]

    const first = renderSnapshot(before, {
      refs,
      documentId: 'main:loader-1',
    })
    refs.beginSnapshot()
    const second = renderSnapshot(after, {
      refs,
      documentId: 'main:loader-1',
    })

    expect(first.text).toBe('- button "A" [ref=e1]\n- link "B" [ref=e2]')
    expect(second.text).toBe(
      [
        '- button "X" [ref=e3]',
        '- button "A" [ref=e1]',
        '- link "B" [ref=e2]',
      ].join('\n'),
    )
  })

  test('marks cursor-augmented non-ARIA nodes actionable', () => {
    const nodes: AXNode[] = [
      ax('1', 'RootWebArea', { children: ['2'] }),
      ax('2', 'generic', { name: name('Fake button'), backendDOMNodeId: 42 }),
    ]
    const out = renderSnapshot(nodes, {
      refs: new RefMap(),
      cursorHits: new Map([[42, ['onclick']]]),
    })
    expect(out.text).toBe('- generic "Fake button" [ref=e1] [cursor=pointer]')
  })

  test('escapes quotes in names', () => {
    const nodes: AXNode[] = [
      ax('1', 'RootWebArea', { children: ['2'] }),
      ax('2', 'button', { name: name('Say "hi"'), backendDOMNodeId: 1 }),
    ]
    expect(renderSnapshot(nodes, { refs: new RefMap() }).text).toBe(
      '- button "Say \\"hi\\"" [ref=e1]',
    )
  })

  test('reports iframe nodes as stitch points and does not ref them', () => {
    const nodes: AXNode[] = [
      ax('1', 'RootWebArea', { children: ['2', '3'] }),
      ax('2', 'button', { name: name('Outer'), backendDOMNodeId: 7 }),
      ax('3', 'Iframe', { backendDOMNodeId: 8 }),
    ]
    const out = renderSnapshot(nodes, { refs: new RefMap() })

    expect(out.text).toBe('- button "Outer" [ref=e1]\n- iframe')
    expect(out.iframes).toEqual([{ backendNodeId: 8, lineIndex: 1, depth: 0 }])
  })

  test('honours baseDepth for spliced child frames', () => {
    const nodes: AXNode[] = [
      ax('1', 'RootWebArea', { children: ['2'] }),
      ax('2', 'button', { name: name('Inner'), backendDOMNodeId: 1 }),
    ]
    expect(
      renderSnapshot(nodes, { refs: new RefMap(), baseDepth: 2 }).text,
    ).toBe('    - button "Inner" [ref=e1]')
  })

  test('keeps a nameless generic when it is cursor-interactive', () => {
    const nodes: AXNode[] = [
      ax('1', 'RootWebArea', { children: ['2'] }),
      ax('2', 'generic', { backendDOMNodeId: 9 }),
    ]
    const out = renderSnapshot(nodes, {
      refs: new RefMap(),
      cursorHits: new Map([[9, ['cursor:pointer']]]),
    })
    expect(out.text).toBe('- generic [ref=e1] [cursor=pointer]')
  })
})
