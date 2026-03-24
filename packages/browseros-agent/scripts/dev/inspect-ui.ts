#!/usr/bin/env bun

// Matches DEV_PORTS.cdp from @browseros/shared/constants/ports
const DEFAULT_CDP_PORT = 9010
const REQUEST_TIMEOUT_MS = 30_000
const EXTENSION_ID =
  process.env.BROWSEROS_EXTENSION_ID || 'bflpfmnmnokmjhmgnolecpppdbdophmk'

// ─── CDP WebSocket Client ────────────────────────────────────────────

type CDPResponse = {
  id: number
  result?: Record<string, unknown>
  error?: { code: number; message: string }
  sessionId?: string
}

type CDPEvent = {
  method: string
  params?: Record<string, unknown>
  sessionId?: string
}

class CDPClient {
  private ws!: WebSocket
  private nextId = 1
  private pending = new Map<
    number,
    {
      resolve: (v: Record<string, unknown>) => void
      reject: (e: Error) => void
      timer: ReturnType<typeof setTimeout>
    }
  >()

  private constructor() {}

  static async connect(port: number): Promise<CDPClient> {
    const client = new CDPClient()
    const versionUrl = `http://127.0.0.1:${port}/json/version`
    let resp: Response
    try {
      resp = await fetch(versionUrl)
    } catch {
      throw new Error(
        `Cannot reach CDP at ${versionUrl}. Is BrowserOS running with --cdp-port=${port}?`,
      )
    }
    const info = (await resp.json()) as { webSocketDebuggerUrl: string }
    let wsUrl = info.webSocketDebuggerUrl
    if (!wsUrl) throw new Error('No webSocketDebuggerUrl in /json/version')
    wsUrl = wsUrl.replace(/ws:\/\/[^/]+/, `ws://127.0.0.1:${port}`)

    return new Promise((resolve, reject) => {
      client.ws = new WebSocket(wsUrl)
      client.ws.onopen = () => resolve(client)
      client.ws.onerror = (e) =>
        reject(new Error(`WebSocket error: ${(e as ErrorEvent).message}`))
      client.ws.onmessage = (ev) => {
        const msg = JSON.parse(String(ev.data)) as CDPResponse | CDPEvent
        if ('id' in msg && msg.id !== undefined) {
          const entry = client.pending.get(msg.id)
          if (entry) {
            client.pending.delete(msg.id)
            clearTimeout(entry.timer)
            if (msg.error) {
              entry.reject(
                new Error(`CDP error ${msg.error.code}: ${msg.error.message}`),
              )
            } else {
              entry.resolve(msg.result ?? {})
            }
          }
        }
      }
      client.ws.onclose = () => {
        for (const [, entry] of client.pending) {
          clearTimeout(entry.timer)
          entry.reject(new Error('WebSocket closed'))
        }
        client.pending.clear()
      }
    })
  }

  send(
    method: string,
    params: Record<string, unknown> = {},
    sessionId?: string,
  ): Promise<Record<string, unknown>> {
    const id = this.nextId++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(
          new Error(
            `CDP request timed out after ${REQUEST_TIMEOUT_MS}ms: ${method}`,
          ),
        )
      }, REQUEST_TIMEOUT_MS)
      this.pending.set(id, { resolve, reject, timer })
      const msg: Record<string, unknown> = { id, method, params }
      if (sessionId) msg.sessionId = sessionId
      this.ws.send(JSON.stringify(msg))
    })
  }

  close() {
    this.ws.close()
  }
}

// ─── Target resolution ───────────────────────────────────────────────

type TargetInfo = {
  targetId: string
  type: string
  title: string
  url: string
}

async function getTargets(cdp: CDPClient): Promise<TargetInfo[]> {
  const result = await cdp.send('Target.getTargets')
  return (result.targetInfos as TargetInfo[]) ?? []
}

function resolveTarget(targets: TargetInfo[], query: string): TargetInfo {
  const idx = Number(query)
  if (!Number.isNaN(idx) && idx >= 0 && idx < targets.length) {
    return targets[idx]
  }
  const q = query.toLowerCase()
  const candidates = query.includes(',')
    ? query.split(',').map((s) => s.trim().toLowerCase())
    : [q]
  for (const c of candidates) {
    const match = targets.find(
      (t) =>
        t.url.toLowerCase().includes(c) || t.title.toLowerCase().includes(c),
    )
    if (match) return match
  }
  throw new Error(`No target matching "${query}"`)
}

async function waitForTargetUrl(
  cdp: CDPClient,
  urlSubstring: string,
  timeoutMs: number,
): Promise<void> {
  const deadline = Date.now() + timeoutMs
  const sub = urlSubstring.toLowerCase()
  while (Date.now() < deadline) {
    const targets = await getTargets(cdp)
    if (targets.some((t) => t.url.toLowerCase().includes(sub))) return
    await new Promise((r) => setTimeout(r, 400))
  }
  throw new Error(`Timeout waiting for target URL containing "${urlSubstring}"`)
}

function findSidepanelTargetIndex(targets: TargetInfo[]): number {
  return targets.findIndex((t) =>
    t.url.toLowerCase().includes('sidepanel.html'),
  )
}

// ─── Session helpers ─────────────────────────────────────────────────

async function attachSession(
  cdp: CDPClient,
  targetId: string,
): Promise<string> {
  const result = await cdp.send('Target.attachToTarget', {
    targetId,
    flatten: true,
  })
  const sessionId = result.sessionId as string
  if (!sessionId) throw new Error('attachToTarget returned no sessionId')
  return sessionId
}

async function enableDomains(
  cdp: CDPClient,
  sessionId: string,
  domains: string[],
): Promise<void> {
  for (const domain of domains) {
    await cdp.send(`${domain}.enable`, {}, sessionId)
  }
}

async function detachSession(cdp: CDPClient, sessionId: string): Promise<void> {
  try {
    await cdp.send('Target.detachFromTarget', { sessionId })
  } catch {
    // already detached
  }
}

// ─── Snapshot: AX tree ───────────────────────────────────────────────

const INTERACTIVE_ROLES = new Set([
  'button',
  'link',
  'textbox',
  'searchbox',
  'textarea',
  'checkbox',
  'radio',
  'combobox',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'tab',
  'switch',
  'slider',
  'spinbutton',
  'option',
  'treeitem',
  'listbox',
])

const SKIP_ROLES = new Set([
  'none',
  'presentation',
  'LineBreak',
  'InlineTextBox',
])

type AXValue = { type: string; value?: string | number | boolean }
type AXProperty = { name: string; value: AXValue }
type AXNode = {
  nodeId: string
  ignored?: boolean
  role?: AXValue
  name?: AXValue
  value?: AXValue
  properties?: AXProperty[]
  childIds?: string[]
  backendDOMNodeId?: number
}

function buildInteractiveTree(nodes: AXNode[]): string[] {
  const nodeMap = new Map<string, AXNode>()
  for (const node of nodes) nodeMap.set(node.nodeId, node)

  const lines: string[] = []

  function walk(nodeId: string): void {
    const node = nodeMap.get(nodeId)
    if (!node) return

    const role = node.ignored
      ? undefined
      : (node.role?.value as string | undefined)
    if (!role || SKIP_ROLES.has(role)) {
      if (node.childIds) for (const childId of node.childIds) walk(childId)
      return
    }

    if (INTERACTIVE_ROLES.has(role) && node.backendDOMNodeId !== undefined) {
      const name = typeof node.name?.value === 'string' ? node.name.value : ''
      const value =
        typeof node.value?.value === 'string' ? node.value.value : ''

      let line = `[${node.backendDOMNodeId}] ${role}`
      if (name) line += ` "${name}"`
      if (
        value &&
        (role === 'textbox' || role === 'searchbox' || role === 'textarea')
      )
        line += ` value="${value}"`
      const props = extractProps(node)
      if (props) line += ` ${props}`
      lines.push(line)
    }

    if (node.childIds) for (const childId of node.childIds) walk(childId)
  }

  const root =
    nodes.find(
      (n) => n.role?.value === 'RootWebArea' || n.role?.value === 'WebArea',
    ) ?? nodes[0]
  if (root?.childIds) for (const childId of root.childIds) walk(childId)

  return lines
}

function extractProps(node: AXNode): string {
  const parts: string[] = []
  if (!node.properties) return ''
  for (const prop of node.properties) {
    if (prop.name === 'checked' && prop.value.value === true)
      parts.push('checked')
    if (prop.name === 'checked' && prop.value.value === 'mixed')
      parts.push('indeterminate')
    if (prop.name === 'disabled' && prop.value.value === true)
      parts.push('disabled')
    if (prop.name === 'expanded' && prop.value.value === true)
      parts.push('expanded')
    if (prop.name === 'expanded' && prop.value.value === false)
      parts.push('collapsed')
    if (prop.name === 'required' && prop.value.value === true)
      parts.push('required')
    if (prop.name === 'selected' && prop.value.value === true)
      parts.push('selected')
    if (prop.name === 'level') parts.push(`level=${prop.value.value}`)
  }
  return parts.length > 0 ? `(${parts.join(', ')})` : ''
}

// ─── Element center: 3-tier fallback ─────────────────────────────────

function quadCenter(q: number[]): { x: number; y: number } {
  const x = ((q[0] ?? 0) + (q[2] ?? 0) + (q[4] ?? 0) + (q[6] ?? 0)) / 4
  const y = ((q[1] ?? 0) + (q[3] ?? 0) + (q[5] ?? 0) + (q[7] ?? 0)) / 4
  return { x, y }
}

async function getElementCenter(
  cdp: CDPClient,
  sessionId: string,
  backendNodeId: number,
): Promise<{ x: number; y: number }> {
  // Tier 1: DOM.getContentQuads
  try {
    const quadsResult = await cdp.send(
      'DOM.getContentQuads',
      { backendNodeId },
      sessionId,
    )
    const quads = quadsResult.quads as number[][] | undefined
    if (quads?.length) {
      const q = quads[0]
      if (q && q.length >= 8) return quadCenter(q)
    }
  } catch {
    // fall through
  }

  // Tier 2: DOM.getBoxModel
  try {
    const boxResult = await cdp.send(
      'DOM.getBoxModel',
      { backendNodeId },
      sessionId,
    )
    const model = boxResult.model as { content?: number[] } | undefined
    const content = model?.content
    if (content && content.length >= 8) return quadCenter(content)
  } catch {
    // fall through
  }

  // Tier 3: getBoundingClientRect via JS
  const resolved = await cdp.send(
    'DOM.resolveNode',
    { backendNodeId },
    sessionId,
  )
  const obj = resolved.object as { objectId?: string } | undefined
  const objectId = obj?.objectId
  if (!objectId)
    throw new Error(
      'Could not resolve element - it may have been removed from the page.',
    )

  const boundsResult = await cdp.send(
    'Runtime.callFunctionOn',
    {
      functionDeclaration:
        'function(){var r=this.getBoundingClientRect();return{x:r.left,y:r.top,w:r.width,h:r.height}}',
      objectId,
      returnByValue: true,
    },
    sessionId,
  )

  const result = boundsResult.result as
    | { value?: { x: number; y: number; w: number; h: number } }
    | undefined
  const rect = result?.value
  if (!rect) throw new Error('Could not get element bounds.')
  return { x: rect.x + rect.w / 2, y: rect.y + rect.h / 2 }
}

// ─── Commands ────────────────────────────────────────────────────────

async function cmdTargets(cdp: CDPClient): Promise<void> {
  const targets = await getTargets(cdp)
  if (targets.length === 0) {
    console.log('No targets found.')
    return
  }
  for (let i = 0; i < targets.length; i++) {
    const t = targets[i]
    const isExtension = t.url.startsWith('chrome-extension://')
    const marker = isExtension ? ' [EXTENSION]' : ''
    console.log(`  ${i}  [${t.type}]  ${t.title || '(untitled)'}${marker}`)
    console.log(`     ${t.url}`)
  }
}

async function cmdScreenshot(
  cdp: CDPClient,
  targetQuery: string,
  outputPath: string,
): Promise<void> {
  const targets = await getTargets(cdp)
  const target = resolveTarget(targets, targetQuery)
  const sessionId = await attachSession(cdp, target.targetId)
  try {
    await enableDomains(cdp, sessionId, ['Page'])
    const result = await cdp.send(
      'Page.captureScreenshot',
      { format: 'png' },
      sessionId,
    )
    const data = result.data as string
    if (!data) throw new Error('No screenshot data returned')
    const buf = Buffer.from(data, 'base64')
    await Bun.write(outputPath, buf)
    console.log(`Screenshot saved to ${outputPath} (${buf.length} bytes)`)
  } finally {
    await detachSession(cdp, sessionId)
  }
}

async function cmdSnapshot(cdp: CDPClient, targetQuery: string): Promise<void> {
  const targets = await getTargets(cdp)
  const target = resolveTarget(targets, targetQuery)
  const sessionId = await attachSession(cdp, target.targetId)
  try {
    await enableDomains(cdp, sessionId, ['Accessibility'])
    const result = await cdp.send('Accessibility.getFullAXTree', {}, sessionId)
    const nodes = (result.nodes as AXNode[]) ?? []
    if (nodes.length === 0) {
      console.log('Empty accessibility tree.')
      return
    }
    const lines = buildInteractiveTree(nodes)
    if (lines.length === 0) {
      console.log('No interactive elements found.')
      return
    }
    console.log(lines.join('\n'))
  } finally {
    await detachSession(cdp, sessionId)
  }
}

async function cmdClick(
  cdp: CDPClient,
  targetQuery: string,
  elementId: number,
): Promise<void> {
  const targets = await getTargets(cdp)
  const target = resolveTarget(targets, targetQuery)
  const sessionId = await attachSession(cdp, target.targetId)
  try {
    await enableDomains(cdp, sessionId, ['DOM', 'Runtime'])
    await cdp.send('DOM.getDocument', { depth: 0 }, sessionId)

    // Scroll into view first
    try {
      await cdp.send(
        'DOM.scrollIntoViewIfNeeded',
        { backendNodeId: elementId },
        sessionId,
      )
    } catch {
      // not critical
    }

    let clicked = false
    try {
      const { x, y } = await getElementCenter(cdp, sessionId, elementId)
      await cdp.send(
        'Input.dispatchMouseEvent',
        { type: 'mouseMoved', x, y },
        sessionId,
      )
      await cdp.send(
        'Input.dispatchMouseEvent',
        { type: 'mousePressed', x, y, button: 'left', clickCount: 1 },
        sessionId,
      )
      await cdp.send(
        'Input.dispatchMouseEvent',
        { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 },
        sessionId,
      )
      clicked = true
      console.log(
        `Clicked element ${elementId} at (${Math.round(x)}, ${Math.round(y)})`,
      )
    } catch (err) {
      console.log(
        `Coordinate click failed (${(err as Error).message}), falling back to JS click`,
      )
    }

    if (!clicked) {
      const resolved = await cdp.send(
        'DOM.resolveNode',
        { backendNodeId: elementId },
        sessionId,
      )
      const obj = resolved.object as { objectId?: string } | undefined
      const objectId = obj?.objectId
      if (!objectId)
        throw new Error('Element not found in DOM. Take a new snapshot.')
      await cdp.send(
        'Runtime.callFunctionOn',
        { functionDeclaration: 'function(){this.click()}', objectId },
        sessionId,
      )
      console.log(`JS-clicked element ${elementId}`)
    }
  } finally {
    await detachSession(cdp, sessionId)
  }
}

async function cmdFill(
  cdp: CDPClient,
  targetQuery: string,
  elementId: number,
  text: string,
): Promise<void> {
  const targets = await getTargets(cdp)
  const target = resolveTarget(targets, targetQuery)
  const sessionId = await attachSession(cdp, target.targetId)
  try {
    await enableDomains(cdp, sessionId, ['DOM', 'Runtime'])
    await cdp.send('DOM.getDocument', { depth: 0 }, sessionId)

    // Scroll into view
    try {
      await cdp.send(
        'DOM.scrollIntoViewIfNeeded',
        { backendNodeId: elementId },
        sessionId,
      )
    } catch {
      // not critical
    }

    // Focus: pushNodesByBackendIdsToFrontend -> DOM.focus
    const pushResult = await cdp.send(
      'DOM.pushNodesByBackendIdsToFrontend',
      { backendNodeIds: [elementId] },
      sessionId,
    )
    const nodeIds = pushResult.nodeIds as number[] | undefined
    if (!nodeIds?.length) throw new Error('Could not push node to frontend')
    await cdp.send('DOM.focus', { nodeId: nodeIds[0] }, sessionId)

    // Clear: Ctrl+A (select all) then Delete
    await cdp.send(
      'Input.dispatchKeyEvent',
      {
        type: 'keyDown',
        key: 'a',
        code: 'KeyA',
        modifiers: 2,
        windowsVirtualKeyCode: 65,
      },
      sessionId,
    )
    await cdp.send(
      'Input.dispatchKeyEvent',
      {
        type: 'keyUp',
        key: 'a',
        code: 'KeyA',
        modifiers: 2,
        windowsVirtualKeyCode: 65,
      },
      sessionId,
    )
    await cdp.send(
      'Input.dispatchKeyEvent',
      {
        type: 'keyDown',
        key: 'Delete',
        code: 'Delete',
        windowsVirtualKeyCode: 46,
      },
      sessionId,
    )
    await cdp.send(
      'Input.dispatchKeyEvent',
      {
        type: 'keyUp',
        key: 'Delete',
        code: 'Delete',
        windowsVirtualKeyCode: 46,
      },
      sessionId,
    )

    // Type via insertText
    await cdp.send('Input.insertText', { text }, sessionId)

    console.log(`Filled element ${elementId} with "${text}"`)
  } finally {
    await detachSession(cdp, sessionId)
  }
}

async function evalReturn(
  cdp: CDPClient,
  targetQuery: string,
  expression: string,
): Promise<unknown> {
  const targets = await getTargets(cdp)
  const target = resolveTarget(targets, targetQuery)
  const sessionId = await attachSession(cdp, target.targetId)
  try {
    await enableDomains(cdp, sessionId, ['Runtime'])
    const result = await cdp.send(
      'Runtime.evaluate',
      { expression, awaitPromise: true, returnByValue: true },
      sessionId,
    )
    const evalResult = result.result as
      | {
          type?: string
          value?: unknown
          description?: string
          subtype?: string
        }
      | undefined
    const exnDetails = result.exceptionDetails as
      | {
          exception?: { description?: string }
        }
      | undefined
    if (exnDetails) {
      throw new Error(
        `JS exception: ${exnDetails.exception?.description ?? 'unknown error'}`,
      )
    }
    if (evalResult?.type === 'undefined') return undefined
    if (evalResult?.value !== undefined) return evalResult.value
    return evalResult?.description ?? evalResult?.type ?? null
  } finally {
    await detachSession(cdp, sessionId)
  }
}

async function cmdEval(
  cdp: CDPClient,
  targetQuery: string,
  expression: string,
): Promise<void> {
  const value = await evalReturn(cdp, targetQuery, expression)
  console.log(
    value === undefined ? 'undefined' : JSON.stringify(value, null, 2),
  )
}

async function reloadTarget(
  cdp: CDPClient,
  targetQuery: string,
): Promise<void> {
  const targets = await getTargets(cdp)
  const target = resolveTarget(targets, targetQuery)
  const sessionId = await attachSession(cdp, target.targetId)
  try {
    await enableDomains(cdp, sessionId, ['Page'])
    await cdp.send('Page.reload', { ignoreCache: false }, sessionId)
  } finally {
    await detachSession(cdp, sessionId)
  }
}

async function cmdE2eSidepanelPersist(cdp: CDPClient): Promise<void> {
  await cmdOpenSidepanel(cdp)

  const refreshSidepanelIndex = async (): Promise<string> => {
    const targets = await getTargets(cdp)
    const idx = findSidepanelTargetIndex(targets)
    if (idx < 0) throw new Error('sidepanel target not in CDP list')
    return String(idx)
  }

  const deadlineSp = Date.now() + 30_000
  let sp: string | undefined
  while (Date.now() < deadlineSp) {
    try {
      sp = await refreshSidepanelIndex()
      break
    } catch {
      await new Promise((r) => setTimeout(r, 400))
    }
  }
  if (!sp) {
    console.error('[e2e] FAIL: sidepanel tab never appeared in CDP targets')
    process.exitCode = 1
    return
  }
  await new Promise((r) => setTimeout(r, 2000))

  const marker = `STRICT-PERSIST-${Date.now()}`
  const sendExpr = `(async() => {
    const marker = ${JSON.stringify(marker)};
    const ta = document.querySelector('textarea');
    if (!ta) return { step: 'no-textarea' };
    const desc = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value');
    if (desc?.set) desc.set.call(ta, marker);
    else ta.value = marker;
    ta.dispatchEvent(new Event('input', { bubbles: true }));
    ta.dispatchEvent(new Event('change', { bubbles: true }));
    const send = [...document.querySelectorAll('button')].find(
      (b) => (b.textContent || '').trim() === 'Send',
    );
    if (!send) return { step: 'no-send' };
    if (send.disabled) return { step: 'send-disabled' };
    send.click();
    return { step: 'sent', marker };
  })()`

  const sendRes = (await evalReturn(cdp, sp, sendExpr)) as {
    step?: string
    marker?: string
  }
  console.log('[e2e] send:', JSON.stringify(sendRes))
  if (sendRes?.step !== 'sent') {
    console.error('[e2e] FAIL: could not send message', sendRes)
    process.exitCode = 1
    return
  }

  const deadlineAssist = Date.now() + 90_000
  let assistantOk = false
  while (Date.now() < deadlineAssist) {
    const check = (await evalReturn(
      cdp,
      sp,
      `(async() => {
        const t = document.body?.innerText || '';
        const m = ${JSON.stringify(marker)};
        const hasUser = t.includes(m);
        const hasAssistant =
          /Thought for|I'm here|How can I help|functioning normally/i.test(t) &&
          hasUser;
        return { hasAssistant, len: t.length };
      })()`,
    )) as { hasAssistant?: boolean; len?: number }
    if (check?.hasAssistant) {
      assistantOk = true
      break
    }
    await new Promise((r) => setTimeout(r, 2000))
  }
  if (!assistantOk) {
    console.error('[e2e] FAIL: assistant reply not detected within timeout')
    process.exitCode = 1
    return
  }

  const findExpr = `(async() => {
    const marker = ${JSON.stringify(marker)};
    const all = await chrome.storage.local.get(null);
    for (const [k, v] of Object.entries(all)) {
      if (!Array.isArray(v)) continue;
      for (const c of v) {
        if (!c || typeof c !== 'object') continue;
        const msgs = c.messages;
        if (!Array.isArray(msgs)) continue;
        if (JSON.stringify(msgs).includes(marker)) {
          return {
            id: c.id,
            storageKey: k,
            messageCount: msgs.length,
          };
        }
      }
    }
    return {
      error: 'conversation not in storage',
      keysSample: Object.keys(all)
        .filter((x) => /conv|local/i.test(x))
        .slice(0, 25),
    };
  })()`

  const found = (await evalReturn(cdp, sp, findExpr)) as {
    id?: string
    error?: string
    keysSample?: string[]
  }
  console.log('[e2e] storage:', JSON.stringify(found))
  if (!found?.id) {
    console.error(
      '[e2e] FAIL: no conversation id in chrome.storage.local',
      found,
    )
    process.exitCode = 1
    return
  }

  const convId = found.id
  await reloadTarget(cdp, sp)
  await waitForTargetUrl(cdp, 'sidepanel.html', 30_000)
  await new Promise((r) => setTimeout(r, 4000))
  sp = await refreshSidepanelIndex()

  const restoreExpr = `(async() => {
    const id = ${JSON.stringify(convId)};
    window.location.hash = '#/?conversationId=' + encodeURIComponent(id);
    return window.location.hash;
  })()`
  const afterHash = await evalReturn(cdp, sp, restoreExpr)
  console.log('[e2e] after restore hash:', afterHash)
  await new Promise((r) => setTimeout(r, 5000))

  const verify = (await evalReturn(
    cdp,
    sp,
    `(async() => {
      const marker = ${JSON.stringify(marker)};
      const t = document.body?.innerText || '';
      return {
        ok: t.includes(marker),
        hash: location.hash,
        len: t.length,
      };
    })()`,
  )) as { ok?: boolean; hash?: string; len?: number }

  console.log('[e2e] verify after reload:', JSON.stringify(verify))
  if (!verify?.ok) {
    console.error(
      '[e2e] FAIL: expected user message missing after reload + restore',
    )
    process.exitCode = 1
    return
  }

  console.log('[e2e] PASS: sidepanel persist + reload + restore')
}

// ─── press_key ────────────────────────────────────────────────────────

const KEY_MAP: Record<string, { code: string; keyCode: number | undefined }> = {
  Backspace: { code: 'Backspace', keyCode: 8 },
  Tab: { code: 'Tab', keyCode: 9 },
  Enter: { code: 'Enter', keyCode: 13 },
  Escape: { code: 'Escape', keyCode: 27 },
  Space: { code: 'Space', keyCode: 32 },
  ' ': { code: 'Space', keyCode: 32 },
  PageUp: { code: 'PageUp', keyCode: 33 },
  PageDown: { code: 'PageDown', keyCode: 34 },
  End: { code: 'End', keyCode: 35 },
  Home: { code: 'Home', keyCode: 36 },
  ArrowLeft: { code: 'ArrowLeft', keyCode: 37 },
  ArrowUp: { code: 'ArrowUp', keyCode: 38 },
  ArrowRight: { code: 'ArrowRight', keyCode: 39 },
  ArrowDown: { code: 'ArrowDown', keyCode: 40 },
  Delete: { code: 'Delete', keyCode: 46 },
  Shift: { code: 'ShiftLeft', keyCode: 16 },
  Control: { code: 'ControlLeft', keyCode: 17 },
  Alt: { code: 'AltLeft', keyCode: 18 },
  Meta: { code: 'MetaLeft', keyCode: 91 },
}

const KEY_ALIASES: Record<string, string> = {
  Return: 'Enter',
  Esc: 'Escape',
  Del: 'Delete',
  Ctrl: 'Control',
  Cmd: 'Meta',
  Command: 'Meta',
  Option: 'Alt',
  Left: 'ArrowLeft',
  Right: 'ArrowRight',
  Up: 'ArrowUp',
  Down: 'ArrowDown',
}

const KEY_TEXT: Record<string, string> = {
  Enter: '\r',
  Tab: '\t',
  Space: ' ',
  ' ': ' ',
}
const MODIFIER_BIT: Record<string, number> = {
  Alt: 1,
  Control: 2,
  Meta: 4,
  Shift: 8,
}

function normalizeKey(key: string): string {
  if (KEY_MAP[key]) return key
  for (const [k, _v] of Object.entries(KEY_MAP)) {
    if (k.toLowerCase() === key.toLowerCase()) return k
  }
  for (const [alias, canonical] of Object.entries(KEY_ALIASES)) {
    if (alias.toLowerCase() === key.toLowerCase()) return canonical
  }
  return key
}

function getKeyInfo(key: string): {
  code: string
  keyCode: number | undefined
} {
  if (KEY_MAP[key]) return KEY_MAP[key]
  if (key.length === 1) {
    if (key >= 'a' && key <= 'z')
      return {
        code: `Key${key.toUpperCase()}`,
        keyCode: key.toUpperCase().charCodeAt(0),
      }
    if (key >= 'A' && key <= 'Z')
      return { code: `Key${key}`, keyCode: key.charCodeAt(0) }
    if (key >= '0' && key <= '9')
      return { code: `Digit${key}`, keyCode: key.charCodeAt(0) }
  }
  return { code: key, keyCode: undefined }
}

async function cmdPressKey(
  cdp: CDPClient,
  targetQuery: string,
  keyCombo: string,
): Promise<void> {
  const targets = await getTargets(cdp)
  const target = resolveTarget(targets, targetQuery)
  const sessionId = await attachSession(cdp, target.targetId)
  try {
    // Parse combo like "Control+A", "Meta+Shift+P", "Enter"
    const parts: string[] = []
    let current = ''
    for (const ch of keyCombo) {
      if (ch === '+' && current) {
        parts.push(current)
        current = ''
      } else current += ch
    }
    if (current) parts.push(current)

    const mainKey = normalizeKey(parts[parts.length - 1])
    const modifiers = parts.slice(0, -1).map(normalizeKey)
    let modBitmask = 0
    for (const mod of modifiers) modBitmask |= MODIFIER_BIT[mod] ?? 0

    // Press modifier keys down
    for (const mod of modifiers) {
      const info = getKeyInfo(mod)
      await cdp.send(
        'Input.dispatchKeyEvent',
        {
          type: 'keyDown',
          key: mod,
          code: info.code,
          windowsVirtualKeyCode: info.keyCode,
        },
        sessionId,
      )
    }

    const mainInfo = getKeyInfo(mainKey)
    const suppressChar = modifiers.some(
      (m) => m === 'Control' || m === 'Alt' || m === 'Meta',
    )
    const text = suppressChar
      ? ''
      : (KEY_TEXT[mainKey] ?? (mainKey.length === 1 ? mainKey : ''))

    await cdp.send(
      'Input.dispatchKeyEvent',
      {
        type: 'keyDown',
        key: mainKey,
        code: mainInfo.code,
        modifiers: modBitmask,
        windowsVirtualKeyCode: mainInfo.keyCode,
        ...(text && { text }),
      },
      sessionId,
    )

    await cdp.send(
      'Input.dispatchKeyEvent',
      {
        type: 'keyUp',
        key: mainKey,
        code: mainInfo.code,
        modifiers: modBitmask,
        windowsVirtualKeyCode: mainInfo.keyCode,
      },
      sessionId,
    )

    // Release modifier keys
    for (const mod of modifiers.reverse()) {
      const info = getKeyInfo(mod)
      await cdp.send(
        'Input.dispatchKeyEvent',
        {
          type: 'keyUp',
          key: mod,
          code: info.code,
        },
        sessionId,
      )
    }

    console.log(`Pressed ${keyCombo}`)
  } finally {
    await detachSession(cdp, sessionId)
  }
}

// ─── scroll ───────────────────────────────────────────────────────────

async function cmdScroll(
  cdp: CDPClient,
  targetQuery: string,
  direction: string,
  amount: number,
): Promise<void> {
  const targets = await getTargets(cdp)
  const target = resolveTarget(targets, targetQuery)
  const sessionId = await attachSession(cdp, target.targetId)
  try {
    await enableDomains(cdp, sessionId, ['Page'])

    const pixels = amount * 120
    const deltaX =
      direction === 'left' ? -pixels : direction === 'right' ? pixels : 0
    const deltaY =
      direction === 'up' ? -pixels : direction === 'down' ? pixels : 0

    if (deltaX === 0 && deltaY === 0) {
      console.error('Direction must be: up, down, left, or right')
      return
    }

    // Get viewport center for scroll position
    const metrics = await cdp.send('Page.getLayoutMetrics', {}, sessionId)
    const viewport = metrics.layoutViewport as {
      clientWidth: number
      clientHeight: number
    }
    const x = viewport.clientWidth / 2
    const y = viewport.clientHeight / 2

    await cdp.send(
      'Input.dispatchMouseEvent',
      {
        type: 'mouseWheel',
        x,
        y,
        deltaX,
        deltaY,
      },
      sessionId,
    )

    console.log(`Scrolled ${direction} by ${amount}`)
  } finally {
    await detachSession(cdp, sessionId)
  }
}

// ─── wait_for ─────────────────────────────────────────────────────────

async function cmdWaitFor(
  cdp: CDPClient,
  targetQuery: string,
  waitType: string,
  waitValue: string,
  timeoutMs: number,
): Promise<void> {
  const targets = await getTargets(cdp)
  const target = resolveTarget(targets, targetQuery)
  const sessionId = await attachSession(cdp, target.targetId)
  try {
    await enableDomains(cdp, sessionId, ['Runtime'])
    const deadline = Date.now() + timeoutMs
    const interval = 500

    while (Date.now() < deadline) {
      let expression: string
      if (waitType === 'text') {
        expression = `document.body?.innerText?.includes(${JSON.stringify(waitValue)}) ?? false`
      } else {
        expression = `!!document.querySelector(${JSON.stringify(waitValue)})`
      }

      const result = await cdp.send(
        'Runtime.evaluate',
        {
          expression,
          returnByValue: true,
        },
        sessionId,
      )

      const evalResult = result.result as { value?: unknown } | undefined
      if (evalResult?.value === true) {
        console.log(`Found ${waitType} "${waitValue}"`)
        return
      }
      await new Promise((r) => setTimeout(r, interval))
    }

    console.error(
      `Timeout: ${waitType} "${waitValue}" not found after ${timeoutMs}ms`,
    )
    process.exitCode = 1
    return
  } finally {
    await detachSession(cdp, sessionId)
  }
}

// ─── hover ────────────────────────────────────────────────────────────

async function cmdHover(
  cdp: CDPClient,
  targetQuery: string,
  elementId: number,
): Promise<void> {
  const targets = await getTargets(cdp)
  const target = resolveTarget(targets, targetQuery)
  const sessionId = await attachSession(cdp, target.targetId)
  try {
    await enableDomains(cdp, sessionId, ['DOM', 'Runtime'])
    await cdp.send('DOM.getDocument', { depth: 0 }, sessionId)

    try {
      await cdp.send(
        'DOM.scrollIntoViewIfNeeded',
        { backendNodeId: elementId },
        sessionId,
      )
    } catch {
      /* not critical */
    }

    const { x, y } = await getElementCenter(cdp, sessionId, elementId)
    await cdp.send(
      'Input.dispatchMouseEvent',
      {
        type: 'mouseMoved',
        x,
        y,
      },
      sessionId,
    )
    console.log(
      `Hovered over element ${elementId} at (${Math.round(x)}, ${Math.round(y)})`,
    )
  } finally {
    await detachSession(cdp, sessionId)
  }
}

// ─── select_option ────────────────────────────────────────────────────

async function cmdSelectOption(
  cdp: CDPClient,
  targetQuery: string,
  elementId: number,
  value: string,
): Promise<void> {
  const targets = await getTargets(cdp)
  const target = resolveTarget(targets, targetQuery)
  const sessionId = await attachSession(cdp, target.targetId)
  try {
    await enableDomains(cdp, sessionId, ['DOM', 'Runtime'])
    await cdp.send('DOM.getDocument', { depth: 0 }, sessionId)

    const resolved = await cdp.send(
      'DOM.resolveNode',
      { backendNodeId: elementId },
      sessionId,
    )
    const objectId = (resolved.object as { objectId?: string })?.objectId
    if (!objectId) throw new Error('Could not resolve element')

    const result = await cdp.send(
      'Runtime.callFunctionOn',
      {
        objectId,
        functionDeclaration: `function(val){
        for(var i=0;i<this.options.length;i++){
          if(this.options[i].value===val||this.options[i].textContent.trim()===val){
            this.selectedIndex=i;
            this.dispatchEvent(new Event('change',{bubbles:true}));
            return this.options[i].textContent.trim();
          }
        }
        return null;
      }`,
        arguments: [{ value }],
        returnByValue: true,
      },
      sessionId,
    )

    const selected = (result.result as { value?: unknown })?.value
    if (selected === null) {
      throw new Error(
        `Option "${value}" not found in select element ${elementId}`,
      )
    }
    console.log(`Selected "${selected}" in element ${elementId}`)
  } finally {
    await detachSession(cdp, sessionId)
  }
}

async function cmdOpenSidepanel(cdp: CDPClient): Promise<void> {
  const targets = await getTargets(cdp)
  const sw = targets.find(
    (t) => t.type === 'service_worker' && t.url.includes(EXTENSION_ID),
  )
  if (!sw) {
    throw new Error(
      `No service worker found for extension ${EXTENSION_ID}. ` +
        'Is the BrowserOS agent extension installed and active?',
    )
  }

  const sessionId = await attachSession(cdp, sw.targetId)
  try {
    await enableDomains(cdp, sessionId, ['Runtime'])
    const result = await cdp.send(
      'Runtime.evaluate',
      {
        expression: `(async () => {
          const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
          const tabId = tabs[0]?.id;
          if (!tabId) throw new Error('No active tab found');
          await chrome.sidePanel.setOptions({ enabled: true, tabId, path: 'sidepanel.html' });
          const result = await chrome.sidePanel.browserosToggle({ tabId });
          return { tabId, ...result };
        })()`,
        awaitPromise: true,
        returnByValue: true,
      },
      sessionId,
    )
    const exnDetails = result.exceptionDetails as
      | {
          exception?: { description?: string }
        }
      | undefined
    if (exnDetails) {
      throw new Error(
        `sidePanel.open() failed: ${exnDetails.exception?.description ?? 'unknown error'}`,
      )
    }
    console.log('Side panel opened.')
  } finally {
    await detachSession(cdp, sessionId)
  }
}

// ─── Help ────────────────────────────────────────────────────────────

function printHelp(): void {
  console.log(`Usage: bun scripts/dev/inspect-ui.ts <command> [args...]

Commands:
  targets                              List all CDP targets (tabs + extensions)
  screenshot <target> [output.png]     Capture screenshot from target
  snapshot <target>                     Print interactive elements with [backendDOMNodeId]
  click <target> <elementId>           Click element by backendDOMNodeId
  fill <target> <elementId> <text>     Focus, clear, and type into element
  press_key <target> <key>             Press key or combo (Enter, Control+A, Meta+Shift+P)
  scroll <target> <direction> [amount] Scroll up/down/left/right (default amount: 3)
  hover <target> <elementId>           Hover over an element
  select_option <target> <id> <value>  Select dropdown option by value or text
  wait_for <target> text|selector <v>  Wait for text or CSS selector (timeout: 10s)
  eval <target> <expression>           Evaluate JS in target context
  open-sidepanel                       Open the BrowserOS agent side panel

Target resolution:
  <target> can be a numeric index from 'targets' output, or a URL/title substring.

Environment:
  BROWSEROS_CDP_PORT   CDP port (default: ${DEFAULT_CDP_PORT})

Examples:
  bun scripts/dev/inspect-ui.ts targets
  bun scripts/dev/inspect-ui.ts screenshot sidepanel /tmp/panel.png
  bun scripts/dev/inspect-ui.ts snapshot app.html
  bun scripts/dev/inspect-ui.ts click sidepanel 42
  bun scripts/dev/inspect-ui.ts fill sidepanel 37 "hello world"
  bun scripts/dev/inspect-ui.ts press_key sidepanel Enter
  bun scripts/dev/inspect-ui.ts press_key sidepanel Control+A
  bun scripts/dev/inspect-ui.ts scroll app.html down 5
  bun scripts/dev/inspect-ui.ts hover sidepanel 69
  bun scripts/dev/inspect-ui.ts select_option app.html 150 "OpenAI"
  bun scripts/dev/inspect-ui.ts wait_for app.html text "Scheduled Tasks"
  bun scripts/dev/inspect-ui.ts wait_for sidepanel selector ".chat-message"
  bun scripts/dev/inspect-ui.ts eval app.html "window.location.hash = '#/settings'"
  bun scripts/dev/inspect-ui.ts open-sidepanel
  e2e-sidepanel-persist                Strict E2E: send chat, reload sidepanel, restore via ?conversationId=`)
}

// ─── Main ────────────────────────────────────────────────────────────

async function main(): Promise<void> {
  const args = process.argv.slice(2)

  if (args.length === 0 || args[0] === '--help' || args[0] === '-h') {
    printHelp()
    process.exit(0)
  }

  const command = args[0]
  const port = Number(process.env.BROWSEROS_CDP_PORT) || DEFAULT_CDP_PORT
  const cdp = await CDPClient.connect(port)

  try {
    switch (command) {
      case 'targets':
        await cmdTargets(cdp)
        break

      case 'screenshot': {
        const target = args[1]
        if (!target) {
          console.error('Usage: screenshot <target> [output.png]')
          process.exit(1)
        }
        const output = args[2] ?? 'screenshot.png'
        await cmdScreenshot(cdp, target, output)
        break
      }

      case 'snapshot': {
        const target = args[1]
        if (!target) {
          console.error('Usage: snapshot <target>')
          process.exit(1)
        }
        await cmdSnapshot(cdp, target)
        break
      }

      case 'click': {
        const target = args[1]
        const elementIdStr = args[2]
        if (!target || !elementIdStr) {
          console.error('Usage: click <target> <elementId>')
          process.exit(1)
        }
        const elementId = Number(elementIdStr)
        if (Number.isNaN(elementId)) {
          console.error(`Invalid elementId: ${elementIdStr}`)
          process.exit(1)
        }
        await cmdClick(cdp, target, elementId)
        break
      }

      case 'fill': {
        const target = args[1]
        const elementIdStr = args[2]
        const text = args.slice(3).join(' ')
        if (!target || !elementIdStr || !text) {
          console.error('Usage: fill <target> <elementId> <text>')
          process.exit(1)
        }
        const elementId = Number(elementIdStr)
        if (Number.isNaN(elementId)) {
          console.error(`Invalid elementId: ${elementIdStr}`)
          process.exit(1)
        }
        await cmdFill(cdp, target, elementId, text)
        break
      }

      case 'press_key': {
        const target = args[1]
        const key = args[2]
        if (!target || !key) {
          console.error('Usage: press_key <target> <key>')
          process.exit(1)
        }
        await cmdPressKey(cdp, target, key)
        break
      }

      case 'scroll': {
        const target = args[1]
        const direction = args[2]
        const amount = Number(args[3] ?? '3')
        if (!target || !direction) {
          console.error('Usage: scroll <target> <up|down|left|right> [amount]')
          process.exit(1)
        }
        await cmdScroll(cdp, target, direction, amount)
        break
      }

      case 'hover': {
        const target = args[1]
        const eid = args[2]
        if (!target || !eid) {
          console.error('Usage: hover <target> <elementId>')
          process.exit(1)
        }
        await cmdHover(cdp, target, Number(eid))
        break
      }

      case 'select_option': {
        const target = args[1]
        const eid = args[2]
        const val = args.slice(3).join(' ')
        if (!target || !eid || !val) {
          console.error('Usage: select_option <target> <elementId> <value>')
          process.exit(1)
        }
        await cmdSelectOption(cdp, target, Number(eid), val)
        break
      }

      case 'wait_for': {
        const target = args[1]
        const waitType = args[2]
        const waitValue = args.slice(3).join(' ')
        if (
          !target ||
          !waitType ||
          !waitValue ||
          !['text', 'selector'].includes(waitType)
        ) {
          console.error('Usage: wait_for <target> text|selector <value>')
          process.exit(1)
        }
        await cmdWaitFor(cdp, target, waitType, waitValue, 10_000)
        break
      }

      case 'eval': {
        const target = args[1]
        const expression = args.slice(2).join(' ')
        if (!target || !expression) {
          console.error('Usage: eval <target> <expression>')
          process.exit(1)
        }
        await cmdEval(cdp, target, expression)
        break
      }

      case 'open-sidepanel':
        await cmdOpenSidepanel(cdp)
        break

      case 'e2e-sidepanel-persist':
        await cmdE2eSidepanelPersist(cdp)
        break

      default:
        console.error(`Unknown command: ${command}`)
        printHelp()
        process.exit(1)
    }
  } finally {
    cdp.close()
  }
}

main().catch((err) => {
  console.error((err as Error).message)
  process.exit(1)
})
