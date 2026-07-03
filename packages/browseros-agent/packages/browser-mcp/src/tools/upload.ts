import { realpathSync } from 'node:fs'
import { resolve, sep } from 'node:path'
import { z } from 'zod'
import { getBrowserosDir } from '../tool-output-dir'
import { defineTool, errorResult, textResult } from './framework'

/**
 * Sensitive path prefixes the upload tool must never touch.
 * Stops prompt-injected instructions from exfiltrating SSH keys,
 * browser credential databases, or shell config files.
 */
const BLOCKED_PATH_PATTERNS = [
  '.ssh',
  '.gnupg',
  '.netrc',
  '.git-credentials',
  'Login Data',
  'Cookies',
  'Web Data',
  'History',
  'Bookmarks',
  'id_rsa',
  'id_ed25519',
  'id_ecdsa',
  'id_dsa',
]

function validateUploadPath(filePath: string): string | null {
  const resolved = resolve(filePath)
  const lower = resolved.toLowerCase()
  for (const pattern of BLOCKED_PATH_PATTERNS) {
    if (lower.includes(pattern.toLowerCase())) {
      return `upload: path "${filePath}" is blocked — it matches a sensitive file pattern`
    }
  }

  // Resolve symlinks on both sides so a symlink inside the browseros dir
  // cannot escape to an arbitrary location on disk.
  let real: string
  try {
    real = realpathSync(resolved)
  } catch {
    return `upload: path "${filePath}" does not exist or cannot be accessed`
  }

  // Require a trailing separator so that a directory named
  // e.g. ".browseros-evil" does not satisfy a prefix check against ".browseros".
  const base = realpathSync(getBrowserosDir())
  const baseWithSep = base.endsWith(sep) ? base : base + sep
  if (real !== base && !real.startsWith(baseWithSep)) {
    return `upload: path "${filePath}" is outside the BrowserOS directory; only files under ${base} may be uploaded`
  }
  return null
}

export const upload = defineTool({
  name: 'upload',
  description:
    'Set local file path(s) on a file input using a ref from the last snapshot. Use for <input type="file"> upload flows; files must exist inside the BrowserOS directory.',
  input: z.object({
    page: z.number().int().describe('Page id from `tabs`.'),
    ref: z
      .string()
      .describe('Ref of the <input type="file"> element, e.g. "e12".'),
    file: z.string().optional().describe('Single local file path to upload.'),
    files: z
      .array(z.string())
      .optional()
      .describe('Local file paths to upload.'),
  }),
  handler: async (args, ctx) => {
    const files = args.files ?? (args.file === undefined ? [] : [args.file])
    if (files.length === 0) {
      return errorResult('upload: provide file or files[].')
    }

    for (const filePath of files) {
      const err = validateUploadPath(filePath)
      if (err) return errorResult(err)
    }

    await ctx.session.input(args.page).uploadFile(args.ref, files)
    return textResult(`Uploaded ${files.length} file(s) to ${args.ref}`, {
      page: args.page,
      ref: args.ref,
      files,
      uploaded: files.length,
    })
  },
})
