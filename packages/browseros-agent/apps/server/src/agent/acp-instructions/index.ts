/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export {
  type EnsureInstructionFileOptions,
  type EnsureInstructionFileResult,
  ensureWorkspaceInstructionFile,
} from './ensureInstructionFile'
export { instructionFilenameFor } from './filenames'
export { promptHash } from './hash'
export {
  findManagedBlock,
  type ManagedBlock,
  renderManagedBlock,
  spliceManagedBlock,
} from './managedBlock'
