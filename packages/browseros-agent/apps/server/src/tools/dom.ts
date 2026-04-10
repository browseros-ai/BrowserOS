import { z } from 'zod'
import { formatSearchResult } from '../browser/dom'
import { defineToolWithCategory } from './framework'
import { writeTempToolOutputFile } from './output-file'

const pageParam = z.number().describe('Page ID (from list_pages)')
const defineObservationTool = defineToolWithCategory('observation')

export const get_dom = defineObservationTool({
  name: 'get_dom',
  description:
    'Get the raw HTML DOM structure of a page or a specific element. Writes outer HTML to a local file and returns the file path. Use a CSS selector to scope to a specific part of the page. For readable text content, prefer get_page_content instead.',
  input: z.object({
    page: pageParam,
    selector: z
      .string()
      .optional()
      .describe(
        "CSS selector to scope (e.g. 'main', '#content', 'form.login')",
      ),
  }),
  output: z.object({
    path: z.string(),
    selector: z.string().optional(),
    totalLength: z.number(),
  }),
  handler: async (args, ctx, response) => {
    const html = await ctx.browser.getDom(args.page, {
      selector: args.selector,
    })

    if (!html) {
      response.error(
        args.selector
          ? `No element found matching "${args.selector}".`
          : 'Page has no DOM content.',
      )
      return
    }

    const path = await writeTempToolOutputFile({
      toolName: 'get-dom',
      extension: 'html',
      content: html,
    })
    response.text(
      args.selector
        ? `Saved DOM for selector "${args.selector}" to ${path}`
        : `Saved DOM to ${path}`,
    )
    response.data({
      path,
      selector: args.selector,
      totalLength: html.length,
    })
  },
})

export const search_dom = defineObservationTool({
  name: 'search_dom',
  description:
    'Search the DOM using plain text, CSS selectors, or XPath queries. Uses the browser\'s native DOM search. Returns matching elements with tag name and attributes. Examples: "Login" (text search), "input[type=email]" (CSS), "//button[@aria-label]" (XPath).',
  input: z.object({
    page: pageParam,
    query: z
      .string()
      .describe('Search query — plain text, CSS selector, or XPath expression'),
    limit: z
      .number()
      .int()
      .min(1)
      .max(200)
      .default(25)
      .describe('Maximum number of results to return (1–200)'),
  }),
  output: z.object({
    query: z.string(),
    totalCount: z.number(),
    shownCount: z.number(),
    results: z.array(
      z.object({
        tag: z.string(),
        nodeId: z.number(),
        backendNodeId: z.number(),
        attributes: z.record(z.string()),
      }),
    ),
  }),
  handler: async (args, ctx, response) => {
    const { results, totalCount } = await ctx.browser.searchDom(
      args.page,
      args.query,
      { limit: args.limit },
    )

    if (results.length === 0) {
      response.text(`No elements matching "${args.query}" found.`)
      response.data({
        query: args.query,
        totalCount,
        shownCount: 0,
        results: [],
      })
      return
    }

    const lines = results.map(formatSearchResult)
    const suffix =
      totalCount > results.length
        ? `\n\n[Showing ${results.length} of ${totalCount} matches. Increase limit to see more.]`
        : ''
    response.text(
      `Found ${totalCount} matching elements:\n\n${lines.join('\n\n')}${suffix}`,
    )
    response.data({
      query: args.query,
      totalCount,
      shownCount: results.length,
      results,
    })
  },
})
