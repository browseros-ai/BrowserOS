import type {
  CladoAction,
  CladoActionResponse,
  RawCladoActionPayload,
} from './types'

/** Parses Clado's structured response plus any raw `<answer>` blocks into executable actions. */
export function parseCladoActions(
  prediction: CladoActionResponse,
): CladoAction[] {
  const actionFromField =
    typeof prediction.action === 'string' ? prediction.action : null

  const rawActions = parseCladoActionsFromRawResponse(prediction.raw_response)
  const primaryFromRaw = rawActions[0] ?? null
  const mergedPrimary = {
    ...primaryFromRaw,
    ...prediction,
    action: actionFromField ?? primaryFromRaw?.action,
  }

  const normalized: CladoAction[] = []
  const primary = normalizeCladoActionPayload(mergedPrimary)
  if (primary) normalized.push(primary)

  for (const candidate of rawActions.slice(1)) {
    const parsed = normalizeCladoActionPayload(candidate)
    if (!parsed) continue
    const prev = normalized[normalized.length - 1]
    if (
      !prev ||
      getCladoActionSignature(prev) !== getCladoActionSignature(parsed)
    ) {
      normalized.push(parsed)
    }
  }

  return normalized
}

function normalizeCladoActionPayload(
  payload: RawCladoActionPayload,
): CladoAction | null {
  if (!payload.action || typeof payload.action !== 'string') {
    return null
  }
  return {
    action: payload.action,
    x: typeof payload.x === 'number' ? payload.x : undefined,
    y: typeof payload.y === 'number' ? payload.y : undefined,
    text: typeof payload.text === 'string' ? payload.text : undefined,
    key: typeof payload.key === 'string' ? payload.key : undefined,
    direction:
      typeof payload.direction === 'string' ? payload.direction : undefined,
    startX: typeof payload.startX === 'number' ? payload.startX : undefined,
    startY: typeof payload.startY === 'number' ? payload.startY : undefined,
    endX: typeof payload.endX === 'number' ? payload.endX : undefined,
    endY: typeof payload.endY === 'number' ? payload.endY : undefined,
    amount: typeof payload.amount === 'number' ? payload.amount : undefined,
    time: typeof payload.time === 'number' ? payload.time : undefined,
    final_answer:
      typeof payload.final_answer === 'string'
        ? payload.final_answer
        : undefined,
  }
}

function parseCladoActionsFromRawResponse(
  rawResponse: string | undefined,
): RawCladoActionPayload[] {
  if (!rawResponse) return []
  const matches = [
    ...rawResponse.matchAll(/<answer>\s*([\s\S]*?)\s*<\/answer>/gi),
  ]
  const parsed: RawCladoActionPayload[] = []
  for (const match of matches) {
    try {
      parsed.push(JSON.parse(match[1]) as RawCladoActionPayload)
    } catch {
      // Ignore malformed answer blocks so one bad block does not drop the whole prediction.
    }
  }
  return parsed
}

export function extractCladoThinking(
  rawResponse: string | undefined,
): string | undefined {
  if (!rawResponse) return undefined
  const matches = [
    ...rawResponse.matchAll(/<thinking>\s*([\s\S]*?)\s*<\/thinking>/gi),
  ]
  if (matches.length === 0) return undefined

  const merged = matches
    .map((match) => match[1]?.replace(/\s+/g, ' ').trim() ?? '')
    .filter((value) => value.length > 0)
    .join(' ')

  if (!merged) return undefined
  return merged
}

export function summarizeCladoPrediction(
  prediction: CladoActionResponse,
): Record<string, unknown> {
  const preview =
    typeof prediction.raw_response === 'string' &&
    prediction.raw_response.length > 0
      ? prediction.raw_response.slice(0, 240)
      : undefined

  return {
    action: prediction.action,
    x: prediction.x,
    y: prediction.y,
    text: prediction.text,
    key: prediction.key,
    direction: prediction.direction,
    startX: prediction.startX,
    startY: prediction.startY,
    endX: prediction.endX,
    endY: prediction.endY,
    amount: prediction.amount,
    time: prediction.time,
    inference_time_seconds: prediction.inference_time_seconds,
    raw_response_preview: preview,
  }
}

export function getCladoActionSignature(action: CladoAction): string {
  switch (action.action) {
    case 'click':
    case 'double_click':
    case 'right_click':
    case 'hover':
      return `${action.action}:${action.x ?? 'x'}:${action.y ?? 'y'}`
    case 'type':
      return `${action.action}:${(action.text ?? '').slice(0, 16)}`
    case 'press_key':
      return `${action.action}:${action.key ?? 'key'}`
    case 'scroll':
      return `${action.action}:${action.direction ?? 'down'}:${action.amount ?? 500}`
    case 'drag':
      return `${action.action}:${action.startX}:${action.startY}:${action.endX}:${action.endY}`
    case 'wait':
      return `${action.action}:${action.time ?? 1}`
    case 'end':
      return action.final_answer
        ? `end(${action.final_answer.slice(0, 32)})`
        : 'end()'
    case 'invalid':
      return `invalid(${(action.text ?? '').slice(0, 40)})`
    default:
      return action.action
  }
}

export function formatCladoHistory(actions: CladoAction[]): string {
  if (actions.length === 0) return 'None'

  const parts = actions.map((action) => {
    switch (action.action) {
      case 'click':
      case 'double_click':
      case 'right_click':
      case 'hover':
        return `${action.action}(${Math.round(action.x ?? 500)}, ${Math.round(action.y ?? 500)})`
      case 'type': {
        const text = (action.text ?? '').replace(/'/g, "\\'")
        return `type('${text}')`
      }
      case 'press_key':
        return `press_key('${action.key ?? 'Enter'}')`
      case 'scroll':
        return `scroll(${action.direction ?? 'down'})`
      case 'drag':
        return `drag(${Math.round(action.startX ?? 500)},${Math.round(action.startY ?? 500)} -> ${Math.round(action.endX ?? 500)},${Math.round(action.endY ?? 500)})`
      case 'wait':
        return `wait(${Math.round(action.time ?? 1)}s)`
      case 'end':
        return 'end()'
      case 'invalid':
        return 'invalid()'
      default:
        return action.action
    }
  })

  return parts.join(' -> ')
}
