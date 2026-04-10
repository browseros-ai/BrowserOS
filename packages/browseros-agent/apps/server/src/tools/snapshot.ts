import { TOOL_LIMITS } from '@browseros/shared/constants/limits'
import { z } from 'zod'
import { defineToolWithCategory } from './framework'
import { writeTempToolOutputFile } from './output-file'

const pageParam = z.number().describe('Page ID (from list_pages)')
const defineObservationTool = defineToolWithCategory('observation')
const defineCaptureTool = defineToolWithCategory('screenshots')
const defineScriptTool = defineToolWithCategory('scripts')

export const take_snapshot = defineObservationTool({
  name: 'take_snapshot',
  description:
    'Get a concise snapshot of interactive elements on the page. Returns a flat list with element IDs (e.g. [47]) that can be used with click, fill, hover, etc. Always take a snapshot before interacting with page elements.',
  input: z.object({
    page: pageParam,
  }),
  output: z.object({
    snapshot: z.string(),
  }),
  handler: async (args, ctx, response) => {
    const tree = await ctx.browser.snapshot(args.page)
    response.text(tree || 'Page has no interactive elements.')
    response.data({ snapshot: tree || '' })
  },
})

export const take_enhanced_snapshot = defineObservationTool({
  name: 'take_enhanced_snapshot',
  description:
    'Get a detailed accessibility tree of the page with structural context (headings, landmarks, dialogs) and cursor-interactive elements that ARIA misses. Use when you need more context than take_snapshot provides.',
  input: z.object({
    page: pageParam,
  }),
  output: z.object({
    snapshot: z.string(),
  }),
  handler: async (args, ctx, response) => {
    const tree = await ctx.browser.enhancedSnapshot(args.page)
    response.text(tree || 'Page has no visible content.')
    response.data({ snapshot: tree || '' })
  },
})

export const get_page_content = defineObservationTool({
  name: 'get_page_content',
  description:
    'Extract page content as clean markdown with headers, links, lists, tables, and formatting preserved. Large results are written to a local file and returned by path. Not for automation — use take_snapshot for that.',
  input: z.object({
    page: pageParam,
    selector: z
      .string()
      .optional()
      .describe(
        "CSS selector to scope extraction (e.g. 'main', '.article-body')",
      ),
    viewportOnly: z
      .boolean()
      .default(false)
      .describe('Only extract content visible in the current viewport'),
    includeLinks: z
      .boolean()
      .default(false)
      .describe('Render links as [text](url) instead of plain text'),
    includeImages: z
      .boolean()
      .default(false)
      .describe('Include image references as ![alt](src)'),
  }),
  output: z.object({
    content: z.string().optional(),
    path: z.string().optional(),
    contentLength: z.number(),
    selector: z.string().optional(),
    viewportOnly: z.boolean(),
    includeLinks: z.boolean(),
    includeImages: z.boolean(),
    writtenToFile: z.boolean(),
  }),
  handler: async (args, ctx, response) => {
    const text = await ctx.browser.contentAsMarkdown(args.page, {
      selector: args.selector,
      viewportOnly: args.viewportOnly,
      includeLinks: args.includeLinks,
      includeImages: args.includeImages,
    })
    if (!text) {
      response.text('No text content found.')
      response.data({
        content: '',
        contentLength: 0,
        selector: args.selector,
        viewportOnly: args.viewportOnly,
        includeLinks: args.includeLinks,
        includeImages: args.includeImages,
        writtenToFile: false,
      })
      return
    }

    if (text.length > TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS) {
      const path = await writeTempToolOutputFile({
        toolName: 'get-page-content',
        extension: 'md',
        content: text,
      })
      // Return truncated content inline so the agent can work immediately,
      // plus the file path for optional deep reading
      const truncated = text.slice(0, TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS)
      response.text(truncated)
      response.text(
        `\n\n[Content truncated at ${TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS} chars. Full content (${text.length} chars) saved to: ${path}]`,
      )
      response.data({
        path,
        contentLength: text.length,
        selector: args.selector,
        viewportOnly: args.viewportOnly,
        includeLinks: args.includeLinks,
        includeImages: args.includeImages,
        writtenToFile: true,
      })
      return
    }

    response.text(text)
    response.data({
      content: text,
      contentLength: text.length,
      selector: args.selector,
      viewportOnly: args.viewportOnly,
      includeLinks: args.includeLinks,
      includeImages: args.includeImages,
      writtenToFile: false,
    })
  },
})

export const take_screenshot = defineCaptureTool({
  name: 'take_screenshot',
  description: 'Take a screenshot of a page',
  input: z.object({
    page: pageParam,
    format: z
      .enum(['png', 'jpeg', 'webp'])
      .default('png')
      .describe('Image format'),
    quality: z
      .number()
      .min(0)
      .max(100)
      .optional()
      .describe('Compression quality (jpeg/webp only)'),
    fullPage: z
      .boolean()
      .default(false)
      .describe('Capture full scrollable page'),
  }),
  output: z.object({
    mimeType: z.string(),
    devicePixelRatio: z.number(),
  }),
  handler: async (args, ctx, response) => {
    const { data, mimeType, devicePixelRatio } = await ctx.browser.screenshot(
      args.page,
      {
        format: args.format,
        quality: args.quality,
        fullPage: args.fullPage,
      },
    )
    response.image(data, mimeType)
    response.text(`devicePixelRatio: ${devicePixelRatio}`)
    response.data({ mimeType, devicePixelRatio })
  },
})

export const get_page_links = defineObservationTool({
  name: 'get_page_links',
  description:
    'Extract all links from the page using the accessibility tree. Returns a deduplicated list of [text](url) entries. More reliable than DOM queries — handles role="link" elements and shadow DOM.',
  input: z.object({
    page: pageParam,
  }),
  output: z.object({
    links: z.array(
      z.object({
        text: z.string(),
        href: z.string(),
      }),
    ),
    count: z.number(),
  }),
  handler: async (args, ctx, response) => {
    const links = await ctx.browser.getPageLinks(args.page)

    if (links.length === 0) {
      response.text('No links found on the page.')
      response.data({ links: [], count: 0 })
      return
    }

    const lines = links.map((l) => (l.text ? `[${l.text}](${l.href})` : l.href))
    response.text(lines.join('\n'))
    response.data({ links, count: links.length })
  },
})

export const evaluate_script = defineScriptTool({
  name: 'evaluate_script',
  description:
    'Execute JavaScript in the page context. Returns the result as a string. Use for reading page state or performing actions not covered by other tools.',
  input: z.object({
    page: pageParam,
    expression: z.string().describe('JavaScript expression to evaluate'),
  }),
  output: z.object({
    text: z.string(),
    value: z.unknown().optional(),
    description: z.string().optional(),
  }),
  handler: async (args, ctx, response) => {
    const result = await ctx.browser.evaluate(args.page, args.expression)

    if (result.error) {
      response.error(`Script error: ${result.error}`)
      return
    }

    const val = result.value
    let text: string
    if (val === undefined) {
      text = result.description ?? 'undefined'
      response.text(text)
    } else if (typeof val === 'string') {
      text = val
      response.text(text)
    } else {
      text = JSON.stringify(val, null, 2)
      response.text(text)
    }
    response.data({
      text,
      value: result.value,
      description: result.description,
    })
  },
})
