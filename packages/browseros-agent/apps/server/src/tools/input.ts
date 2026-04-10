import { z } from 'zod'
import { defineToolWithCategory } from './framework'

const pageParam = z.number().describe('Page ID (from list_pages)')
const defineInputTool = defineToolWithCategory('input')
const elementParam = z
  .number()
  .describe('Element ID from snapshot (the number in [N])')

export const click = defineInputTool({
  name: 'click',
  description: 'Click an element by its ID from the last snapshot',
  input: z.object({
    page: pageParam,
    element: elementParam,
    button: z
      .enum(['left', 'right', 'middle'])
      .default('left')
      .describe('Mouse button'),
    clickCount: z
      .number()
      .default(1)
      .describe('Number of clicks (2 for double-click)'),
  }),
  output: z.object({
    action: z.literal('click'),
    page: z.number(),
    element: z.number(),
    button: z.enum(['left', 'right', 'middle']),
    clickCount: z.number(),
  }),
  handler: async (args, ctx, response) => {
    const coords = await ctx.browser.click(args.page, args.element, {
      button: args.button,
      clickCount: args.clickCount,
    })
    const coordText = coords
      ? ` at (${Math.round(coords.x)}, ${Math.round(coords.y)})`
      : ''
    response.text(`Clicked [${args.element}]${coordText}`)
    response.data({
      action: 'click',
      page: args.page,
      element: args.element,
      button: args.button,
      clickCount: args.clickCount,
    })
    response.includeSnapshot(args.page)
  },
})

export const click_at = defineInputTool({
  name: 'click_at',
  description: 'Click at specific page coordinates',
  input: z.object({
    page: pageParam,
    x: z.number().describe('X coordinate'),
    y: z.number().describe('Y coordinate'),
    button: z
      .enum(['left', 'right', 'middle'])
      .default('left')
      .describe('Mouse button'),
    clickCount: z.number().default(1).describe('Number of clicks'),
  }),
  output: z.object({
    action: z.literal('click_at'),
    page: z.number(),
    x: z.number(),
    y: z.number(),
    button: z.enum(['left', 'right', 'middle']),
    clickCount: z.number(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.clickAt(args.page, args.x, args.y, {
      button: args.button,
      clickCount: args.clickCount,
    })
    response.text(`Clicked at (${args.x}, ${args.y})`)
    response.data({
      action: 'click_at',
      page: args.page,
      x: args.x,
      y: args.y,
      button: args.button,
      clickCount: args.clickCount,
    })
    response.includeSnapshot(args.page)
  },
})

export const hover_at = defineInputTool({
  name: 'hover_at',
  description: 'Hover at specific page coordinates',
  input: z.object({
    page: pageParam,
    x: z.number().describe('X coordinate'),
    y: z.number().describe('Y coordinate'),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.hoverAt(args.page, args.x, args.y)
    response.text(`Hovered at (${args.x}, ${args.y})`)
    response.includeSnapshot(args.page)
  },
})

export const type_at = defineInputTool({
  name: 'type_at',
  description:
    'Click at specific coordinates then type text. Use for typing into inputs at known positions.',
  input: z.object({
    page: pageParam,
    x: z.number().describe('X coordinate to click before typing'),
    y: z.number().describe('Y coordinate to click before typing'),
    text: z.string().describe('Text to type'),
    clear: z.boolean().default(false).describe('Clear field before typing'),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.typeAt(args.page, args.x, args.y, args.text, args.clear)
    response.text(`Typed ${args.text.length} chars at (${args.x}, ${args.y})`)
    response.includeSnapshot(args.page)
  },
})

export const drag_at = defineInputTool({
  name: 'drag_at',
  description: 'Drag from one coordinate to another',
  input: z.object({
    page: pageParam,
    startX: z.number().describe('Start X coordinate'),
    startY: z.number().describe('Start Y coordinate'),
    endX: z.number().describe('End X coordinate'),
    endY: z.number().describe('End Y coordinate'),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.dragAt(
      args.page,
      { x: args.startX, y: args.startY },
      { x: args.endX, y: args.endY },
    )
    response.text(
      `Dragged from (${args.startX}, ${args.startY}) to (${args.endX}, ${args.endY})`,
    )
    response.includeSnapshot(args.page)
  },
})

export const hover = defineInputTool({
  name: 'hover',
  description: 'Hover over an element by its ID',
  input: z.object({
    page: pageParam,
    element: elementParam,
  }),
  output: z.object({
    action: z.literal('hover'),
    page: z.number(),
    element: z.number(),
  }),
  handler: async (args, ctx, response) => {
    const coords = await ctx.browser.hover(args.page, args.element)
    response.text(
      `Hovered over [${args.element}] at (${Math.round(coords.x)}, ${Math.round(coords.y)})`,
    )
    response.data({ action: 'hover', page: args.page, element: args.element })
  },
})

export const clear = defineInputTool({
  name: 'clear',
  description: 'Clear the text content of an input or textarea element',
  input: z.object({
    page: pageParam,
    element: elementParam,
  }),
  output: z.object({
    action: z.literal('clear'),
    page: z.number(),
    element: z.number(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.fill(args.page, args.element, '', true)
    response.text(`Cleared [${args.element}]`)
    response.data({ action: 'clear', page: args.page, element: args.element })
    response.includeSnapshot(args.page)
  },
})

export const fill = defineInputTool({
  name: 'fill',
  description:
    'Type text into an input or textarea element. Focuses the element, optionally clears existing text, then types character by character.',
  input: z.object({
    page: pageParam,
    element: elementParam,
    text: z.string().describe('Text to type'),
    clear: z
      .boolean()
      .default(true)
      .describe('Clear existing text before typing'),
  }),
  output: z.object({
    action: z.literal('fill'),
    page: z.number(),
    element: z.number(),
    textLength: z.number(),
    clear: z.boolean(),
  }),
  handler: async (args, ctx, response) => {
    const coords = await ctx.browser.fill(
      args.page,
      args.element,
      args.text,
      args.clear,
    )
    const coordText = coords
      ? ` at (${Math.round(coords.x)}, ${Math.round(coords.y)})`
      : ''
    response.text(
      `Typed ${args.text.length} characters into [${args.element}]${coordText}`,
    )
    response.data({
      action: 'fill',
      page: args.page,
      element: args.element,
      textLength: args.text.length,
      clear: args.clear,
    })
    response.includeSnapshot(args.page)
  },
})

export const press_key = defineInputTool({
  name: 'press_key',
  description:
    "Press a key or key combination (e.g. 'Enter', 'Escape', 'Control+A', 'Meta+Shift+P'). Sent to the currently focused element.",
  input: z.object({
    page: pageParam,
    key: z
      .string()
      .describe("Key or combo like 'Enter', 'Control+A', 'ArrowDown'"),
  }),
  output: z.object({
    action: z.literal('press_key'),
    page: z.number(),
    key: z.string(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.pressKey(args.page, args.key)
    response.text(`Pressed ${args.key}`)
    response.data({ action: 'press_key', page: args.page, key: args.key })
  },
})

export const drag = defineInputTool({
  name: 'drag',
  description:
    'Drag from one element to another element or to specific coordinates',
  input: z.object({
    page: pageParam,
    sourceElement: elementParam.describe('Element ID to drag from'),
    targetElement: z.number().optional().describe('Element ID to drag to'),
    targetX: z
      .number()
      .optional()
      .describe('Target X coordinate (if not using targetElement)'),
    targetY: z
      .number()
      .optional()
      .describe('Target Y coordinate (if not using targetElement)'),
  }),
  output: z.object({
    action: z.literal('drag'),
    page: z.number(),
    sourceElement: z.number(),
    targetElement: z.number().optional(),
    targetX: z.number().optional(),
    targetY: z.number().optional(),
  }),
  handler: async (args, ctx, response) => {
    const coords = await ctx.browser.drag(args.page, args.sourceElement, {
      element: args.targetElement,
      x: args.targetX,
      y: args.targetY,
    })
    const target =
      args.targetElement !== undefined
        ? `[${args.targetElement}]`
        : `(${args.targetX}, ${args.targetY})`
    response.text(
      `Dragged [${args.sourceElement}] (${Math.round(coords.from.x)}, ${Math.round(coords.from.y)}) \u2192 ${target} (${Math.round(coords.to.x)}, ${Math.round(coords.to.y)})`,
    )
    response.data({
      action: 'drag',
      page: args.page,
      sourceElement: args.sourceElement,
      targetElement: args.targetElement,
      targetX: args.targetX,
      targetY: args.targetY,
    })
    response.includeSnapshot(args.page)
  },
})

export const scroll = defineInputTool({
  name: 'scroll',
  description: 'Scroll the page or a specific element',
  input: z.object({
    page: pageParam,
    direction: z
      .enum(['up', 'down', 'left', 'right'])
      .default('down')
      .describe('Scroll direction'),
    amount: z.number().default(3).describe('Number of scroll ticks'),
    element: z
      .number()
      .optional()
      .describe('Element ID to scroll at (scrolls page center if omitted)'),
  }),
  output: z.object({
    action: z.literal('scroll'),
    page: z.number(),
    direction: z.enum(['up', 'down', 'left', 'right']),
    amount: z.number(),
    element: z.number().optional(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.scroll(
      args.page,
      args.direction,
      args.amount,
      args.element,
    )
    response.text(`Scrolled ${args.direction} by ${args.amount}`)
    response.data({
      action: 'scroll',
      page: args.page,
      direction: args.direction,
      amount: args.amount,
      element: args.element,
    })
  },
})

export const handle_dialog = defineInputTool({
  name: 'handle_dialog',
  description: 'Accept or dismiss a JavaScript dialog (alert, confirm, prompt)',
  input: z.object({
    page: pageParam,
    accept: z.boolean().describe('true to accept, false to dismiss'),
    promptText: z
      .string()
      .optional()
      .describe('Text to enter for prompt dialogs'),
  }),
  output: z.object({
    action: z.literal('handle_dialog'),
    page: z.number(),
    accept: z.boolean(),
    promptText: z.string().optional(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.handleDialog(args.page, args.accept, args.promptText)
    response.text(args.accept ? 'Dialog accepted' : 'Dialog dismissed')
    response.data({
      action: 'handle_dialog',
      page: args.page,
      accept: args.accept,
      promptText: args.promptText,
    })
  },
})

export const focus = defineInputTool({
  name: 'focus',
  description: 'Focus an element by its ID. Scrolls into view first.',
  input: z.object({
    page: pageParam,
    element: elementParam,
  }),
  output: z.object({
    action: z.literal('focus'),
    page: z.number(),
    element: z.number(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.focus(args.page, args.element)
    response.text(`Focused [${args.element}]`)
    response.data({ action: 'focus', page: args.page, element: args.element })
  },
})

export const check = defineInputTool({
  name: 'check',
  description: 'Check a checkbox or radio button. No-op if already checked.',
  input: z.object({
    page: pageParam,
    element: elementParam,
  }),
  output: z.object({
    action: z.literal('check'),
    page: z.number(),
    element: z.number(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.check(args.page, args.element)
    response.text(`Checked [${args.element}]`)
    response.data({ action: 'check', page: args.page, element: args.element })
    response.includeSnapshot(args.page)
  },
})

export const uncheck = defineInputTool({
  name: 'uncheck',
  description: 'Uncheck a checkbox. No-op if already unchecked.',
  input: z.object({
    page: pageParam,
    element: elementParam,
  }),
  output: z.object({
    action: z.literal('uncheck'),
    page: z.number(),
    element: z.number(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.uncheck(args.page, args.element)
    response.text(`Unchecked [${args.element}]`)
    response.data({ action: 'uncheck', page: args.page, element: args.element })
    response.includeSnapshot(args.page)
  },
})

export const upload_file = defineInputTool({
  name: 'upload_file',
  description:
    'Set file(s) on a file input element. Files must be absolute paths on disk.',
  input: z.object({
    page: pageParam,
    element: elementParam.describe(
      'Element ID of the <input type="file"> element',
    ),
    files: z.array(z.string()).describe('Absolute file paths to upload'),
  }),
  output: z.object({
    action: z.literal('upload_file'),
    page: z.number(),
    element: z.number(),
    files: z.array(z.string()),
    fileCount: z.number(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.uploadFile(args.page, args.element, args.files)
    response.text(`Set ${args.files.length} file(s) on [${args.element}]`)
    response.data({
      action: 'upload_file',
      page: args.page,
      element: args.element,
      files: args.files,
      fileCount: args.files.length,
    })
    response.includeSnapshot(args.page)
  },
})

export const select_option = defineInputTool({
  name: 'select_option',
  description:
    'Select an option in a <select> dropdown by value or visible text',
  input: z.object({
    page: pageParam,
    element: elementParam.describe('Element ID of the <select> element'),
    value: z.string().describe('Option value or visible text to select'),
  }),
  output: z.object({
    action: z.literal('select_option'),
    page: z.number(),
    element: z.number(),
    value: z.string(),
    selected: z.string(),
  }),
  handler: async (args, ctx, response) => {
    const selected = await ctx.browser.selectOption(
      args.page,
      args.element,
      args.value,
    )
    if (selected === null) {
      response.error(
        `Option "${args.value}" not found in select [${args.element}]. Use take_snapshot to see available options.`,
      )
      return
    }
    response.text(`Selected "${selected}" in [${args.element}]`)
    response.data({
      action: 'select_option',
      page: args.page,
      element: args.element,
      value: args.value,
      selected,
    })
    response.includeSnapshot(args.page)
  },
})
