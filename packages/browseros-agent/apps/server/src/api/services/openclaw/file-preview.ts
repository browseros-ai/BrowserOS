/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Helpers used by the `/claw/files/:id/preview` and
 * `/claw/files/:id/download` routes:
 *
 *   - MIME-type detection (extension first, magic-byte fallback for
 *     ambiguous extensions).
 *   - Bounded text-snippet reader for inline previews.
 *   - Image bytes reader for the rail's thumbnails.
 *
 * No streaming code lives here — the download route streams via Hono
 * directly. This module only handles the small in-memory reads the
 * preview UX needs.
 */

import { open, stat } from 'node:fs/promises'
import { extname } from 'node:path'

/** Hard cap on the inline text snippet returned by the preview API. */
export const TEXT_PREVIEW_MAX_BYTES = 1 * 1024 * 1024 // 1 MB

/** Hard cap on inline image bytes returned as a base64 data URL. */
export const IMAGE_PREVIEW_MAX_BYTES = 4 * 1024 * 1024 // 4 MB

const MIME_BY_EXTENSION: Record<string, string> = {
  '.txt': 'text/plain',
  '.md': 'text/markdown',
  '.markdown': 'text/markdown',
  '.json': 'application/json',
  '.jsonl': 'application/x-ndjson',
  '.csv': 'text/csv',
  '.tsv': 'text/tab-separated-values',
  '.xml': 'application/xml',
  '.yaml': 'application/yaml',
  '.yml': 'application/yaml',
  '.toml': 'application/toml',
  '.ini': 'text/plain',
  '.log': 'text/plain',
  '.html': 'text/html',
  '.htm': 'text/html',
  '.css': 'text/css',
  '.js': 'text/javascript',
  '.mjs': 'text/javascript',
  '.cjs': 'text/javascript',
  '.ts': 'text/typescript',
  '.tsx': 'text/typescript',
  '.jsx': 'text/javascript',
  '.py': 'text/x-python',
  '.rb': 'text/x-ruby',
  '.go': 'text/x-go',
  '.rs': 'text/x-rust',
  '.java': 'text/x-java',
  '.kt': 'text/x-kotlin',
  '.swift': 'text/x-swift',
  '.c': 'text/x-c',
  '.h': 'text/x-c',
  '.cpp': 'text/x-c++',
  '.hpp': 'text/x-c++',
  '.sh': 'application/x-sh',
  '.zsh': 'application/x-sh',
  '.bash': 'application/x-sh',
  '.sql': 'application/sql',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.webp': 'image/webp',
  '.bmp': 'image/bmp',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
  '.heic': 'image/heic',
  '.heif': 'image/heif',
  '.pdf': 'application/pdf',
  '.zip': 'application/zip',
  '.tar': 'application/x-tar',
  '.gz': 'application/gzip',
  '.tgz': 'application/gzip',
  '.bz2': 'application/x-bzip2',
  '.7z': 'application/x-7z-compressed',
  '.mp3': 'audio/mpeg',
  '.wav': 'audio/wav',
  '.ogg': 'audio/ogg',
  '.mp4': 'video/mp4',
  '.webm': 'video/webm',
  '.mov': 'video/quicktime',
  '.docx':
    'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  '.xlsx': 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  '.pptx':
    'application/vnd.openxmlformats-officedocument.presentationml.presentation',
}

/**
 * Magic-byte signatures for cases where the extension is missing or
 * misleading. Only covers the formats whose preview path differs from
 * the default binary path (text vs image vs PDF vs other).
 */
const MAGIC_BYTE_SIGNATURES: Array<{
  mime: string
  matches: (head: Uint8Array) => boolean
}> = [
  {
    mime: 'image/png',
    matches: (h) =>
      h[0] === 0x89 &&
      h[1] === 0x50 &&
      h[2] === 0x4e &&
      h[3] === 0x47 &&
      h[4] === 0x0d &&
      h[5] === 0x0a,
  },
  {
    mime: 'image/jpeg',
    matches: (h) => h[0] === 0xff && h[1] === 0xd8 && h[2] === 0xff,
  },
  {
    mime: 'image/gif',
    matches: (h) =>
      h[0] === 0x47 && h[1] === 0x49 && h[2] === 0x46 && h[3] === 0x38,
  },
  {
    mime: 'image/webp',
    matches: (h) =>
      h[0] === 0x52 &&
      h[1] === 0x49 &&
      h[2] === 0x46 &&
      h[3] === 0x46 &&
      h[8] === 0x57 &&
      h[9] === 0x45 &&
      h[10] === 0x42 &&
      h[11] === 0x50,
  },
  {
    mime: 'application/pdf',
    matches: (h) =>
      h[0] === 0x25 && h[1] === 0x50 && h[2] === 0x44 && h[3] === 0x46,
  },
]

const MAGIC_BYTE_PROBE_LEN = 12

/**
 * Best-effort MIME detection. Tries the extension map first, then
 * falls back to magic-byte sniffing for the formats whose preview
 * path differs from the default binary handling. Returns
 * `application/octet-stream` when we can't tell.
 */
export async function detectMimeType(absolutePath: string): Promise<string> {
  const fromExtension = MIME_BY_EXTENSION[extname(absolutePath).toLowerCase()]
  if (fromExtension) return fromExtension

  let head: Uint8Array
  try {
    const handle = await open(absolutePath, 'r')
    try {
      const buffer = new Uint8Array(MAGIC_BYTE_PROBE_LEN)
      const { bytesRead } = await handle.read(
        buffer,
        0,
        MAGIC_BYTE_PROBE_LEN,
        0,
      )
      head = buffer.subarray(0, bytesRead)
    } finally {
      await handle.close()
    }
  } catch {
    return 'application/octet-stream'
  }

  for (const sig of MAGIC_BYTE_SIGNATURES) {
    if (sig.matches(head)) return sig.mime
  }

  if (looksLikeText(head)) return 'text/plain'
  return 'application/octet-stream'
}

export type PreviewKind = 'text' | 'image' | 'pdf' | 'binary' | 'missing'

export interface BasePreview {
  kind: PreviewKind
  mimeType: string
  size: number
  mtimeMs: number
}

export interface TextPreview extends BasePreview {
  kind: 'text'
  snippet: string
  /** True when the on-disk file is larger than `TEXT_PREVIEW_MAX_BYTES`. */
  truncated: boolean
}

export interface ImagePreview extends BasePreview {
  kind: 'image'
  /** Base64 data URL (incl. `data:` prefix) suitable for `<img src>`. */
  dataUrl: string
}

export interface PdfPreview extends BasePreview {
  kind: 'pdf'
}

export interface BinaryPreview extends BasePreview {
  kind: 'binary'
}

export interface MissingPreview {
  kind: 'missing'
}

export type FilePreview =
  | TextPreview
  | ImagePreview
  | PdfPreview
  | BinaryPreview
  | MissingPreview

/**
 * Build a preview payload for the inline-card / rail preview Sheet.
 * Reads at most `TEXT_PREVIEW_MAX_BYTES` (text) or
 * `IMAGE_PREVIEW_MAX_BYTES` (image) into memory; everything else
 * returns a metadata-only `binary` preview and the UI offers a
 * download instead.
 */
export async function buildFilePreview(
  absolutePath: string,
): Promise<FilePreview> {
  let stats: Awaited<ReturnType<typeof stat>>
  try {
    stats = await stat(absolutePath)
  } catch {
    return { kind: 'missing' }
  }

  const mimeType = await detectMimeType(absolutePath)
  const base = {
    mimeType,
    size: stats.size,
    mtimeMs: stats.mtimeMs,
  } as const

  if (mimeType === 'application/pdf') {
    return { kind: 'pdf', ...base }
  }

  if (isTextMime(mimeType)) {
    return readTextPreview(absolutePath, base)
  }

  if (isImageMime(mimeType)) {
    return readImagePreview(absolutePath, base)
  }

  return { kind: 'binary', ...base }
}

async function readTextPreview(
  absolutePath: string,
  base: { mimeType: string; size: number; mtimeMs: number },
): Promise<TextPreview> {
  const handle = await open(absolutePath, 'r')
  try {
    const length = Math.min(base.size, TEXT_PREVIEW_MAX_BYTES)
    const buffer = new Uint8Array(length)
    const { bytesRead } = await handle.read(buffer, 0, length, 0)
    const snippet = new TextDecoder('utf-8', { fatal: false }).decode(
      buffer.subarray(0, bytesRead),
    )
    return {
      kind: 'text',
      ...base,
      snippet,
      truncated: base.size > TEXT_PREVIEW_MAX_BYTES,
    }
  } finally {
    await handle.close()
  }
}

async function readImagePreview(
  absolutePath: string,
  base: { mimeType: string; size: number; mtimeMs: number },
): Promise<ImagePreview | BinaryPreview> {
  if (base.size > IMAGE_PREVIEW_MAX_BYTES) {
    // Too big to inline — let the user download.
    return { kind: 'binary', ...base }
  }
  const handle = await open(absolutePath, 'r')
  try {
    const buffer = new Uint8Array(base.size)
    await handle.read(buffer, 0, base.size, 0)
    const dataUrl = `data:${base.mimeType};base64,${Buffer.from(buffer).toString('base64')}`
    return { kind: 'image', ...base, dataUrl }
  } finally {
    await handle.close()
  }
}

function isTextMime(mime: string): boolean {
  if (mime.startsWith('text/')) return true
  return (
    mime === 'application/json' ||
    mime === 'application/x-ndjson' ||
    mime === 'application/xml' ||
    mime === 'application/yaml' ||
    mime === 'application/toml' ||
    mime === 'application/sql' ||
    mime === 'application/x-sh'
  )
}

function isImageMime(mime: string): boolean {
  return mime.startsWith('image/') && mime !== 'image/svg+xml'
  // SVG is text — let it go through the text path so users can read
  // markup, not view a base64 blob.
}

/**
 * Crude text-vs-binary heuristic for files whose extension and magic
 * bytes both fail to identify them. Counts NUL bytes — text files
 * essentially never contain them; binaries usually do.
 */
function looksLikeText(head: Uint8Array): boolean {
  if (head.length === 0) return true
  let nulCount = 0
  for (const byte of head) {
    if (byte === 0) nulCount += 1
  }
  return nulCount === 0
}
