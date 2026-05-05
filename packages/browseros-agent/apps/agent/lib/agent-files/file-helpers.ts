/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure helpers used by the artifact card and the Outputs rail.
 * Display formatting only — no React, no fetch, no DOM. Anything
 * stateful belongs in `./useAgentOutputs` or `./useFilePreview`.
 */

import { buildAgentApiUrl } from '@/entrypoints/app/agents/agent-api-url'

/**
 * Coarse classification of a file's intended preview / icon path.
 * Mirrors the server-side `FilePreviewKind` minus `missing` — the
 * client only ever computes a kind for a row it already has.
 */
export type FileKind = 'text' | 'image' | 'pdf' | 'binary'

const TEXT_EXTENSIONS = new Set([
  'txt',
  'md',
  'markdown',
  'json',
  'jsonl',
  'csv',
  'tsv',
  'xml',
  'yaml',
  'yml',
  'toml',
  'ini',
  'log',
  'html',
  'htm',
  'css',
  'js',
  'mjs',
  'cjs',
  'ts',
  'tsx',
  'jsx',
  'py',
  'rb',
  'go',
  'rs',
  'java',
  'kt',
  'swift',
  'c',
  'h',
  'cpp',
  'hpp',
  'sh',
  'zsh',
  'bash',
  'sql',
  'svg',
])

const IMAGE_EXTENSIONS = new Set([
  'png',
  'jpg',
  'jpeg',
  'gif',
  'webp',
  'bmp',
  'ico',
  'heic',
  'heif',
])

/** Best-effort kind based on extension only. Server's preview API
 * is the source of truth for actual rendering — this is just for
 * picking an icon / sort hint without a network round-trip. */
export function inferFileKind(path: string): FileKind {
  const ext = extensionOf(path).toLowerCase()
  if (ext === 'pdf') return 'pdf'
  if (IMAGE_EXTENSIONS.has(ext)) return 'image'
  if (TEXT_EXTENSIONS.has(ext)) return 'text'
  return 'binary'
}

/** Plain extension without the leading dot. Empty string when none. */
export function extensionOf(path: string): string {
  const dot = path.lastIndexOf('.')
  if (dot === -1) return ''
  const slash = path.lastIndexOf('/')
  if (dot < slash) return ''
  return path.slice(dot + 1)
}

/** File name (final path segment), no directory prefix. */
export function basenameOf(path: string): string {
  const slash = path.lastIndexOf('/')
  return slash === -1 ? path : path.slice(slash + 1)
}

const SIZE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'] as const

/** "2.4 MB" / "340 KB" / "78 B" — for the artifact card's right-side
 *  metadata. Not localised; the rail uses one space + the unit. */
export function formatFileSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—'
  if (bytes < 1024) return `${bytes} ${SIZE_UNITS[0]}`
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < SIZE_UNITS.length - 1) {
    value /= 1024
    unit += 1
  }
  // 1-digit precision below 10, integer above — feels less noisy.
  const formatted = value < 10 ? value.toFixed(1) : Math.round(value).toString()
  return `${formatted} ${SIZE_UNITS[unit]}`
}

/**
 * Build the per-file download URL using the same agent-api root the
 * rest of the harness hits. Returned URL is already absolute.
 */
export function buildFileDownloadUrl(baseUrl: string, fileId: string): string {
  return buildAgentApiUrl(
    baseUrl,
    `/files/${encodeURIComponent(fileId)}/download`,
  )
}
