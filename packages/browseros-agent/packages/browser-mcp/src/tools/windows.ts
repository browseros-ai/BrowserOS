import type { WindowInfo } from '@browseros/browser-core/core/windows'
import { z } from 'zod'
import { defineTool, errorResult, textResult } from './framework'

const ACTIONS = [
  'list',
  'create',
  'close',
  'activate',
  'set_visibility',
] as const

export const windows = defineTool({
  name: 'windows',
  description:
    'Manage browser windows: list windows, create visible or hidden windows, close or activate a window, and show or hide windows.',
  input: z.object({
    action: z.enum(ACTIONS).default('list'),
    windowId: z
      .number()
      .int()
      .optional()
      .describe('Window id for close, activate, and set_visibility.'),
    hidden: z
      .boolean()
      .default(false)
      .describe('Create a hidden window for action="create".'),
    visible: z
      .boolean()
      .optional()
      .describe('Target visibility for action="set_visibility".'),
    activate: z
      .boolean()
      .optional()
      .describe('Focus the window after making it visible.'),
  }),
  annotations: { openWorldHint: true },
  handler: async (args, ctx) => {
    switch (args.action) {
      case 'list': {
        const all = await ctx.session.windows.list()
        return textResult(formatWindowList(all), {
          action: 'list',
          windows: all,
          count: all.length,
        })
      }
      case 'create': {
        const window = await ctx.session.windows.create({ hidden: args.hidden })
        const hiddenMarker = !window.isVisible ? ' (hidden)' : ''
        return textResult(`created window ${window.windowId}${hiddenMarker}`, {
          action: 'create',
          window,
        })
      }
      case 'close': {
        if (args.windowId === undefined) {
          return errorResult('windows close: windowId is required.')
        }
        await ctx.session.windows.close(args.windowId)
        return textResult(`closed window ${args.windowId}`, {
          action: 'close',
          windowId: args.windowId,
        })
      }
      case 'activate': {
        if (args.windowId === undefined) {
          return errorResult('windows activate: windowId is required.')
        }
        await ctx.session.windows.activate(args.windowId)
        return textResult(`activated window ${args.windowId}`, {
          action: 'activate',
          windowId: args.windowId,
        })
      }
      case 'set_visibility': {
        if (args.windowId === undefined) {
          return errorResult('windows set_visibility: windowId is required.')
        }
        if (args.visible === undefined) {
          return errorResult('windows set_visibility: visible is required.')
        }
        const result = await ctx.session.windows.setVisibility(args.windowId, {
          visible: args.visible,
          activate: args.activate,
        })
        const state = result.window.isVisible ? 'visible' : 'hidden'
        return textResult(
          `set window ${result.previousWindowId} ${state}; new window id ${result.newWindowId}`,
          {
            action: 'set_visibility',
            previousWindowId: result.previousWindowId,
            newWindowId: result.newWindowId,
            replaced: result.replaced,
            window: result.window,
          },
        )
      }
      default:
        return errorResult('windows: unsupported action.')
    }
  },
})

function formatWindowList(windows: WindowInfo[]): string {
  if (windows.length === 0) return 'No windows found.'

  const lines = [`Found ${windows.length} windows:`, '']
  for (const window of windows) {
    const markers: string[] = []
    if (!window.isVisible) markers.push('HIDDEN')
    if (window.isActive) markers.push('ACTIVE')
    const suffix = markers.length > 0 ? ` [${markers.join(', ')}]` : ''
    lines.push(
      `Window ${window.windowId} (${window.windowType}, ${window.tabCount} tabs)${suffix}`,
    )
  }
  return lines.join('\n')
}
