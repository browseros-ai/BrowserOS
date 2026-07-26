import type { WindowInfo } from '@browseros/browser-core/core/windows'
import { z } from 'zod'
import { defineTool, errorResult, textResult } from './framework'

const ACTIONS = ['list', 'create', 'close', 'activate'] as const

export const windows = defineTool({
  name: 'windows',
  description:
    'Manage browser windows: list, create, close, or activate a window.',
  input: z.object({
    action: z.enum(ACTIONS).default('list'),
    windowId: z
      .number()
      .int()
      .optional()
      .describe('Window id for close and activate.'),
  }),
  annotations: {
    title: 'Manage windows',
    destructiveHint: true,
    openWorldHint: true,
  },
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
        const window = await ctx.session.windows.create()
        return textResult(`created window ${window.windowId}`, {
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
      default:
        return errorResult('windows: unsupported action.')
    }
  },
})

function formatWindowList(windows: WindowInfo[]): string {
  if (windows.length === 0) return 'No windows found.'

  const lines = [`Found ${windows.length} windows:`, '']
  for (const window of windows) {
    const suffix = window.isActive ? ' [ACTIVE]' : ''
    lines.push(
      `Window ${window.windowId} (${window.windowType}, ${window.tabCount} tabs)${suffix}`,
    )
  }
  return lines.join('\n')
}
