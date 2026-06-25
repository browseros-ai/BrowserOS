import { mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import { z } from 'zod'
import { getToolOutputDir } from '../tool-output-dir'
import { defineTool, textResult } from './framework'
import { recordBrowserOutputFile } from './output-file'

export const download = defineTool({
  name: 'download',
  description:
    'Click an element (by ref from the last snapshot) to trigger a file download, and save it to a BrowserOS output file. Returns the saved path and filename.',
  input: z.object({
    page: z.number().int().describe('Page id from `tabs`.'),
    ref: z
      .string()
      .describe('Ref of the element that triggers the download, e.g. "e12".'),
  }),
  handler: async (args, ctx) => {
    const { session } = await ctx.session.pages.getSession(args.page)
    // A fresh subdir avoids Chromium filename uniquifying on repeated downloads.
    const dir = await mkdtemp(join(await getToolOutputDir(), 'download-'))

    const { suggestedFilename } = await captureDownload(session, dir, () =>
      ctx.session.input(args.page).click(args.ref),
    )
    const path = join(dir, suggestedFilename)
    recordBrowserOutputFile(path)
    return textResult(`Downloaded "${suggestedFilename}" to: ${path}`, {
      page: args.page,
      ref: args.ref,
      path,
      filename: suggestedFilename,
    })
  },
})

/** Arms page download capture, runs the trigger, and resolves when the download completes. */
async function captureDownload(
  session: ProtocolApi,
  downloadPath: string,
  trigger: () => Promise<void>,
): Promise<{ suggestedFilename: string }> {
  await session.Page.setDownloadBehavior({ behavior: 'allow', downloadPath })

  return new Promise<{ suggestedFilename: string }>((resolve, reject) => {
    let guid = ''
    let suggestedFilename = ''

    const timer = setTimeout(() => {
      cleanup()
      reject(new Error(`Download timed out after ${TIMEOUTS.DOWNLOAD}ms`))
    }, TIMEOUTS.DOWNLOAD)

    const unsubscribeBegin = session.Page.on('downloadWillBegin', (params) => {
      guid = params.guid
      suggestedFilename = params.suggestedFilename
    })

    const unsubscribeProgress = session.Page.on(
      'downloadProgress',
      (params) => {
        if (params.guid !== guid) return
        if (params.state === 'completed') {
          cleanup()
          resolve({ suggestedFilename })
        } else if (params.state === 'canceled') {
          cleanup()
          reject(new Error('Download was canceled'))
        }
      },
    )

    const cleanup = () => {
      clearTimeout(timer)
      unsubscribeBegin()
      unsubscribeProgress()
      session.Page.setDownloadBehavior({ behavior: 'default' }).catch(() => {})
    }

    trigger().catch((err) => {
      cleanup()
      reject(err instanceof Error ? err : new Error(String(err)))
    })
  })
}
