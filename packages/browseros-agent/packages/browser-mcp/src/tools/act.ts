import type { BrowserSession } from '@browseros/browser-core/core/session'
import { z } from 'zod'
import {
  defineTool,
  errorResult,
  type ToolResult,
  textResult,
} from './framework'

type InputApi = ReturnType<BrowserSession['input']>

// Flat (not discriminated-union) schema: some providers reject nested anyOf JSON Schema. The kind
// is validated at runtime in the handler. All page mutation goes through this one tool.
export const act = defineTool({
  name: 'act',
  description:
    'Act on the page using refs from the last snapshot. kinds: click, type (into focused element), fill (one field via ref+value, or many via fields[]), press (a key/combo), hover, focus, check, uncheck, select (an option value), scroll, drag. Reads back a diff of what changed - re-snapshot if you need fresh refs.',
  input: z.object({
    page: z.number().int(),
    kind: z.enum([
      'click',
      'click_at',
      'type',
      'type_at',
      'fill',
      'press',
      'hover',
      'hover_at',
      'focus',
      'check',
      'uncheck',
      'select',
      'scroll',
      'drag',
      'drag_at',
    ]),
    ref: z.string().optional().describe('Target element ref, e.g. "e12".'),
    text: z.string().optional().describe('Text for kind=type.'),
    value: z.string().optional().describe('Value for kind=fill/select.'),
    fields: z
      .array(z.object({ ref: z.string(), value: z.string() }))
      .optional()
      .describe('Multiple fields for kind=fill, filled in order.'),
    key: z
      .string()
      .optional()
      .describe('Key/combo for kind=press, e.g. "Enter", "Control+a".'),
    direction: z.enum(['up', 'down', 'left', 'right']).optional(),
    amount: z
      .number()
      .optional()
      .describe('Scroll amount (wheel notches), default 3.'),
    x: z.number().optional().describe('Viewport x coordinate for *_at kinds.'),
    y: z.number().optional().describe('Viewport y coordinate for *_at kinds.'),
    targetRef: z.string().optional().describe('Target ref for kind=drag.'),
    startX: z.number().optional().describe('Drag start x coordinate.'),
    startY: z.number().optional().describe('Drag start y coordinate.'),
    endX: z.number().optional().describe('Drag end x coordinate.'),
    endY: z.number().optional().describe('Drag end y coordinate.'),
    button: z.enum(['left', 'middle', 'right']).optional(),
    clickCount: z.number().int().optional(),
    clear: z.boolean().optional(),
  }),
  handler: async (args, ctx, response) => {
    const input = ctx.session.input(args.page)

    const err = await runKind(args, input)
    if (err) return err

    response.data({ kind: args.kind })
    response.includeDiff(args.page, { includeStructured: true })
    return textResult(`ok (${args.kind})`)
  },
})

type ActArgs = {
  kind: string
  ref?: string
  text?: string
  value?: string
  fields?: { ref: string; value: string }[]
  key?: string
  direction?: 'up' | 'down' | 'left' | 'right'
  amount?: number
  x?: number
  y?: number
  targetRef?: string
  startX?: number
  startY?: number
  endX?: number
  endY?: number
  button?: 'left' | 'middle' | 'right'
  clickCount?: number
  clear?: boolean
}

type ActHandler = (
  args: ActArgs,
  input: InputApi,
) => Promise<ToolResult | undefined>

const ACT_HANDLERS: Record<string, ActHandler> = {
  click: clickRef,
  click_at: clickAt,
  type: typeFocused,
  type_at: typeAt,
  fill,
  press,
  hover,
  hover_at: hoverAt,
  focus,
  check,
  uncheck,
  select,
  scroll,
  drag,
  drag_at: dragAt,
}

async function runKind(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  const handler = ACT_HANDLERS[args.kind]
  return handler
    ? handler(args, input)
    : errorResult(`act: unknown kind "${args.kind}".`)
}

async function clickRef(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  if (!args.ref) return errorResult('act click: ref is required.')
  await input.click(args.ref, clickOptions(args))
  return undefined
}

async function clickAt(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  const point = pointFromArgs(args, 'click_at')
  if ('content' in point) return point
  await input.clickAt(point.x, point.y, clickOptions(args))
  return undefined
}

async function typeFocused(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  if (args.text === undefined) return errorResult('act type: text is required.')
  await input.type(args.text)
  return undefined
}

async function typeAt(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  const point = pointFromArgs(args, 'type_at')
  if ('content' in point) return point
  if (args.text === undefined)
    return errorResult('act type_at: text is required.')
  await input.typeAt(point.x, point.y, args.text, args.clear ?? false)
  return undefined
}

async function fill(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  if (args.fields) {
    for (const field of args.fields)
      await input.fill(field.ref, field.value, { clear: args.clear })
    return undefined
  }
  if (args.ref && args.value !== undefined) {
    await input.fill(args.ref, args.value, { clear: args.clear })
    return undefined
  }
  return errorResult('act fill: provide fields[] or both ref and value.')
}

async function press(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  if (!args.key) return errorResult('act press: key is required.')
  await input.press(args.key)
  return undefined
}

async function hover(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  if (!args.ref) return errorResult('act hover: ref is required.')
  await input.hover(args.ref)
  return undefined
}

async function hoverAt(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  const point = pointFromArgs(args, 'hover_at')
  if ('content' in point) return point
  await input.hoverAt(point.x, point.y)
  return undefined
}

async function focus(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  if (!args.ref) return errorResult('act focus: ref is required.')
  await input.focus(args.ref)
  return undefined
}

async function check(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  if (!args.ref) return errorResult('act check: ref is required.')
  await input.check(args.ref)
  return undefined
}

async function uncheck(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  if (!args.ref) return errorResult('act uncheck: ref is required.')
  await input.uncheck(args.ref)
  return undefined
}

async function select(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  if (!args.ref || args.value === undefined) {
    return errorResult('act select: ref and value are required.')
  }
  await input.selectOption(args.ref, args.value)
  return undefined
}

async function scroll(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  await input.scroll(args.direction ?? 'down', args.amount ?? 3, args.ref)
  return undefined
}

async function drag(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  if (!args.ref || !args.targetRef) {
    return errorResult('act drag: ref and targetRef are required.')
  }
  await input.drag(args.ref, args.targetRef)
  return undefined
}

async function dragAt(
  args: ActArgs,
  input: InputApi,
): Promise<ToolResult | undefined> {
  if (
    args.startX === undefined ||
    args.startY === undefined ||
    args.endX === undefined ||
    args.endY === undefined
  ) {
    return errorResult(
      'act drag_at: startX, startY, endX, and endY are required.',
    )
  }
  await input.dragAt(
    { x: args.startX, y: args.startY },
    { x: args.endX, y: args.endY },
  )
  return undefined
}

function pointFromArgs(
  args: ActArgs,
  kind: string,
): { x: number; y: number } | ToolResult {
  if (args.x === undefined || args.y === undefined) {
    return errorResult(`act ${kind}: x and y are required.`)
  }
  return { x: args.x, y: args.y }
}

function clickOptions(args: ActArgs): { button?: string; clickCount?: number } {
  return {
    ...(args.button && { button: args.button }),
    ...(args.clickCount !== undefined && { clickCount: args.clickCount }),
  }
}
