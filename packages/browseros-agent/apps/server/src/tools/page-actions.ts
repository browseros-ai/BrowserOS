import { mkdir, mkdtemp, rename, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { z } from 'zod'
import { defineToolWithCategory, resolveWorkingPath } from './framework'

const pageParam = z.number().describe('Page ID (from list_pages)')
const defineCaptureTool = defineToolWithCategory('screenshots')
const elementParam = z
  .number()
  .describe('Element ID from snapshot (the number in [N])')

export const save_pdf = defineCaptureTool({
  name: 'save_pdf',
  description: 'Save the current page as a PDF file',
  input: z.object({
    page: pageParam,
    path: z.string().describe('File path for the PDF (e.g. "report.pdf")'),
    cwd: z
      .string()
      .optional()
      .describe(
        'Working directory to resolve relative paths against; defaults to the execution directory',
      ),
  }),
  output: z.object({
    action: z.literal('save_pdf'),
    page: z.number(),
    path: z.string(),
  }),
  handler: async (args, ctx, response) => {
    const resolvedPath = resolveWorkingPath(ctx, args.path, args.cwd)
    const { data } = await ctx.browser.printToPDF(args.page)
    await Bun.write(resolvedPath, Buffer.from(data, 'base64'))
    response.text(`Saved PDF to ${resolvedPath}`)
    response.data({
      action: 'save_pdf',
      page: args.page,
      path: resolvedPath,
    })
  },
})

export const save_screenshot = defineCaptureTool({
  name: 'save_screenshot',
  description: 'Take a screenshot of a page and save it to a file on disk',
  input: z.object({
    page: pageParam,
    path: z
      .string()
      .describe('File path for the screenshot (e.g. "screenshot.png")'),
    cwd: z
      .string()
      .optional()
      .describe(
        'Working directory to resolve relative paths against; defaults to the execution directory',
      ),
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
    action: z.literal('save_screenshot'),
    page: z.number(),
    path: z.string(),
    format: z.enum(['png', 'jpeg', 'webp']),
    quality: z.number().optional(),
    fullPage: z.boolean(),
  }),
  handler: async (args, ctx, response) => {
    const resolvedPath = resolveWorkingPath(ctx, args.path, args.cwd)
    const { data } = await ctx.browser.screenshot(args.page, {
      format: args.format,
      quality: args.quality,
      fullPage: args.fullPage,
    })
    await Bun.write(resolvedPath, Buffer.from(data, 'base64'))
    response.text(`Saved screenshot to ${resolvedPath}`)
    response.data({
      action: 'save_screenshot',
      page: args.page,
      path: resolvedPath,
      format: args.format,
      quality: args.quality,
      fullPage: args.fullPage,
    })
  },
})

export const download_file = defineCaptureTool({
  name: 'download_file',
  description:
    'Click an element to trigger a file download and save it to disk',
  input: z.object({
    page: pageParam,
    element: elementParam.describe('Element ID that triggers the download'),
    path: z.string().describe('Directory to save the downloaded file into'),
    cwd: z
      .string()
      .optional()
      .describe(
        'Working directory to resolve relative paths against; defaults to the execution directory',
      ),
  }),
  output: z.object({
    action: z.literal('download_file'),
    page: z.number(),
    element: z.number(),
    directory: z.string(),
    suggestedFilename: z.string(),
    destinationPath: z.string(),
  }),
  handler: async (args, ctx, response) => {
    const resolvedDir = resolveWorkingPath(ctx, args.path, args.cwd)
    const baseDir = ctx.directories.workingDir ?? tmpdir()
    await mkdir(baseDir, { recursive: true })
    const tempDir = await mkdtemp(join(baseDir, 'browseros-dl-'))

    try {
      const { filePath, suggestedFilename } =
        await ctx.browser.downloadViaClick(args.page, args.element, tempDir)

      const destPath = join(resolvedDir, suggestedFilename)
      await rename(filePath, destPath)

      response.text(`Downloaded "${suggestedFilename}" to ${destPath}`)
      response.data({
        action: 'download_file',
        page: args.page,
        element: args.element,
        directory: resolvedDir,
        suggestedFilename,
        destinationPath: destPath,
      })
    } finally {
      await rm(tempDir, { recursive: true, force: true }).catch(() => {})
    }
  },
})
