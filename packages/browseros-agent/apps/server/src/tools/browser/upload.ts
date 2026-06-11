import { z } from 'zod'
import {
  defineTool,
  textResult,
  type ToolResult,
} from './framework'

/**
 * Upload files to a file input element (<input type="file">) using CDP DOM.setFileInputFiles.
 * This bypasses the native file picker dialog, enabling automated file uploads.
 *
 * Usage:
 *  1. Take a snapshot to get the ref of the <input type="file"> element.
 *  2. Call this tool with the ref and an array of file paths.
 *  3. The files must be accessible to the Chromium browser process.
 */
export const upload = defineTool({
  name: 'upload',
  description:
    'Upload files to a file input element without opening the file picker dialog. ' +
    'Takes a ref (from snapshot) pointing to an <input type="file"> element and an array of file paths. ' +
    'The files will be set on the input, after which you can click the submit button to upload. ' +
    'Requires that the file paths are accessible to the browser process.',
  input: z.object({
    page: z.number().int().describe('The page id from tabs.'),
    ref: z
      .string()
      .describe(
        'The ref of the file input element (from snapshot). ' +
          'Must point to an <input type="file"> element.',
      ),
    files: z
      .array(z.string())
      .describe(
        'Array of file paths to upload. ' +
          'Paths must be accessible to the Chromium browser process. ' +
          'Supports multiple files for <input multiple>.',
      ),
  }),
  annotations: {
    readOnlyHint: false,
    destructiveHint: false,
    openWorldHint: false,
  },
  handler: async (args, ctx): Promise<ToolResult> => {
    await ctx.session.input(args.page).upload(args.ref, args.files)
    const fileList = args.files.map((f) => `"${f}"`).join(', ')
    return textResult(
      `ok (upload) · set ${args.files.length} file(s): ${fileList}`,
    )
  },
})
