/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Wire types shared by the inline artifact card and the per-agent
 * Outputs rail. These mirror `ProducedFileEntry` /
 * `ProducedFilesRailGroup` on the server and the `FilePreview`
 * discriminated union from `apps/server/src/api/services/openclaw/file-preview.ts`.
 *
 * The schema mirror is deliberate (vs sharing a workspace package)
 * because the server keeps the on-disk row shape — `agentDefinitionId`,
 * `sessionKey` — out of the wire payload. Dropping those columns at the
 * type boundary keeps the client honest about what it can refer to.
 */

export interface ProducedFile {
  id: string
  /** Workspace-relative POSIX path. */
  path: string
  size: number
  mtimeMs: number
  /** Server clock when the file was first attributed to its turn. */
  createdAt: number
  detectedBy: 'diff' | 'tool'
}

export interface ProducedFilesRailGroup {
  turnId: string
  /** First non-blank line of the user prompt that initiated this turn. */
  turnPrompt: string
  createdAt: number
  files: ProducedFile[]
}

export type FilePreviewKind = 'text' | 'image' | 'pdf' | 'binary' | 'missing'

interface BasePreview {
  kind: FilePreviewKind
  mimeType: string
  size: number
  mtimeMs: number
}

export interface TextFilePreview extends BasePreview {
  kind: 'text'
  snippet: string
  /** True when the on-disk file is larger than the server's snippet cap. */
  truncated: boolean
}

export interface ImageFilePreview extends BasePreview {
  kind: 'image'
  /** Base64 data URL (incl. `data:` prefix). Suitable for `<img src>`. */
  dataUrl: string
}

export interface PdfFilePreview extends BasePreview {
  kind: 'pdf'
}

export interface BinaryFilePreview extends BasePreview {
  kind: 'binary'
}

export interface MissingFilePreview {
  kind: 'missing'
}

export type FilePreview =
  | TextFilePreview
  | ImageFilePreview
  | PdfFilePreview
  | BinaryFilePreview
  | MissingFilePreview
