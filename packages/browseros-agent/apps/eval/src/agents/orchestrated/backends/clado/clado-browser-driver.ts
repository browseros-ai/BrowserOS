import {
  CLADO_PAGE_SCOPED_TOOLS,
  type CladoActionPoint,
  type CladoViewport,
} from './types'

function clampCladoNormalizedCoordinate(value: number): number {
  return Math.min(999, Math.max(0, Math.round(value)))
}

/** Converts Clado's 0-1000 normalized coordinate space into BrowserOS viewport pixels. */
export function resolveCladoPoint(
  viewport: CladoViewport,
  normalizedX: number | undefined,
  normalizedY: number | undefined,
): CladoActionPoint {
  const nx = clampCladoNormalizedCoordinate(normalizedX ?? 500)
  const ny = clampCladoNormalizedCoordinate(normalizedY ?? 500)

  return {
    x: Math.round((nx / 1000) * viewport.width),
    y: Math.round((ny / 1000) * viewport.height),
  }
}

/** Adapts Clado action tool arguments to the BrowserOS MCP tool argument contract. */
export function prepareCladoToolArgs(
  toolName: string,
  args: Record<string, unknown>,
  pageId: number,
): Record<string, unknown> {
  const prepared: Record<string, unknown> = { ...args }

  if (
    toolName === 'evaluate_script' &&
    typeof prepared.function === 'string' &&
    prepared.expression === undefined
  ) {
    prepared.expression = toCladoEvaluateExpression(prepared.function)
    delete prepared.function
  }

  if (
    toolName === 'click_at' &&
    typeof prepared.dblClick === 'boolean' &&
    prepared.clickCount === undefined
  ) {
    prepared.clickCount = prepared.dblClick ? 2 : 1
    delete prepared.dblClick
  }

  if (
    CLADO_PAGE_SCOPED_TOOLS.has(toolName) &&
    typeof prepared.page !== 'number'
  ) {
    prepared.page = pageId
  }

  return prepared
}

export function prepareCladoToolCall(
  toolName: string,
  args: Record<string, unknown>,
  pageId: number,
): { toolName: string; args: Record<string, unknown> } {
  const prepared = prepareCladoToolArgs(toolName, args, pageId)
  const page = prepared.page

  switch (toolName) {
    case 'take_screenshot':
      return {
        toolName: 'screenshot',
        args: {
          page,
          ...(typeof prepared.fullPage === 'boolean' && {
            fullPage: prepared.fullPage,
          }),
        },
      }
    case 'evaluate_script': {
      const expression = toCladoEvaluateExpression(
        prepared.expression ?? prepared.function,
      )
      return {
        toolName: 'run',
        args: { page, code: `return await (${expression})` },
      }
    }
    case 'click_at':
      return {
        toolName: 'act',
        args: {
          page,
          kind: 'click_at',
          x: prepared.x,
          y: prepared.y,
          button: prepared.button,
          clickCount: prepared.clickCount,
        },
      }
    case 'hover_at':
      return {
        toolName: 'act',
        args: { page, kind: 'hover_at', x: prepared.x, y: prepared.y },
      }
    case 'type_at':
      return {
        toolName: 'act',
        args: {
          page,
          kind: 'type_at',
          x: prepared.x,
          y: prepared.y,
          text: prepared.text,
          clear: prepared.clear,
        },
      }
    case 'press_key':
      return {
        toolName: 'act',
        args: { page, kind: 'press', key: prepared.key },
      }
    case 'scroll':
      return {
        toolName: 'act',
        args: {
          page,
          kind: 'scroll',
          direction: prepared.direction,
          amount: prepared.amount,
        },
      }
    case 'drag_at':
      return {
        toolName: 'act',
        args: {
          page,
          kind: 'drag_at',
          startX: prepared.startX,
          startY: prepared.startY,
          endX: prepared.endX,
          endY: prepared.endY,
        },
      }
    case 'navigate_page':
      return {
        toolName: 'navigate',
        args: {
          page,
          action: prepared.action ?? 'url',
          url: prepared.url,
        },
      }
    case 'close_page':
      return { toolName: 'tabs', args: { action: 'close', page } }
    case 'wait_for':
      return {
        toolName: 'wait',
        args: {
          page,
          for: prepared.selector ? 'selector' : 'time',
          ...(prepared.selector !== undefined && { value: prepared.selector }),
          timeout: prepared.timeout,
        },
      }
    default:
      return { toolName, args: prepared }
  }
}

function toCladoEvaluateExpression(rawFunction: unknown): string {
  const source = String(rawFunction).trim()
  if (source.startsWith('() =>') || source.startsWith('async () =>')) {
    return `(${source})()`
  }
  if (source.startsWith('function')) {
    return `(${source})()`
  }
  return source
}

export function normalizeCladoPressKey(key: string | undefined): string {
  const raw = (key ?? '').trim()
  if (!raw) throw new Error('press_key action missing key field')

  const map: Record<string, string> = {
    'C-a': 'Control+A',
    'C-c': 'Control+C',
    'C-v': 'Control+V',
    'C-x': 'Control+X',
    'C-z': 'Control+Z',
    'C-y': 'Control+Y',
    'C-s': 'Control+S',
    'C-t': 'Control+T',
    'C-w': 'Control+W',
    'C-h': 'Control+H',
    'C-f': 'Control+F',
    'C-+': 'Control++',
    'C--': 'Control+-',
    'C-tab': 'Control+Tab',
    'C-S-tab': 'Control+Shift+Tab',
    'C-S-n': 'Control+Shift+N',
    'C-down': 'Control+ArrowDown',
    'M-a': 'Meta+A',
    'M-c': 'Meta+C',
    'M-v': 'Meta+V',
    'M-x': 'Meta+X',
    'M-f4': 'Alt+F4',
  }
  return map[raw] ?? raw
}

export function normalizeCladoDirection(
  direction: string | undefined,
): 'up' | 'down' | 'left' | 'right' {
  if (
    direction === 'up' ||
    direction === 'down' ||
    direction === 'left' ||
    direction === 'right'
  ) {
    return direction
  }
  return 'down'
}

export function normalizeCladoScrollAmount(amount: number | undefined): number {
  if (typeof amount !== 'number') return 500
  if (amount <= 0) return 100
  const clamped = Math.min(amount, 1000)
  return Math.max(100, Math.round((clamped / 1000) * 900))
}
