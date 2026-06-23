import { z } from 'zod'
import { defineTool, textResult } from './framework'
import { writeTempToolOutputBinaryFile } from './output-file'

export const pdf = defineTool({
  name: 'pdf',
  description:
    'Print the page to a PDF and save it to a BrowserOS output file, returning the path. Use for archiving or reading a page as a document; prefer read for extracting text.',
  input: z.object({
    page: z.number().int().describe('Page id from `tabs`.'),
    landscape: z.boolean().optional().describe('Use landscape orientation.'),
    background: z
      .boolean()
      .optional()
      .describe('Compatibility alias for printBackground.'),
    printBackground: z
      .boolean()
      .optional()
      .describe('Print background graphics.'),
    preferCSSPageSize: z
      .boolean()
      .default(false)
      .describe('Use CSS page size when the page defines one.'),
  }),
  annotations: { readOnlyHint: true },
  handler: async (args, ctx) => {
    const { session } = await ctx.session.pages.getSession(args.page)
    const { data } = await session.Page.printToPDF({
      landscape: args.landscape ?? false,
      printBackground: args.printBackground ?? args.background ?? true,
      preferCSSPageSize: args.preferCSSPageSize,
    })
    const bytes = Buffer.from(data, 'base64')
    const path = await writeTempToolOutputBinaryFile({
      toolName: 'pdf',
      extension: 'pdf',
      content: bytes,
    })
    return textResult(
      `Saved page ${args.page} as PDF (${bytes.length} bytes) to: ${path}`,
      {
        page: args.page,
        path,
        bytes: bytes.length,
      },
    )
  },
})
