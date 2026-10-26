/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  ClaudeDesktopExtension,
  ClaudeDesktopInstallResult,
} from '@browseros/claw-api'
import { createMutation, createQuery } from 'react-query-kit'
import { apiClient } from './client'

export const useClaudeDesktopExtension = createQuery<ClaudeDesktopExtension>({
  queryKey: ['api', 'claude-desktop', 'extension'],
  fetcher: async () => (await apiClient()).getClaudeDesktopExtension(),
})

/** Opens the extension in Claude, which then asks the person to confirm. */
export const useInstallClaudeDesktopExtension =
  createMutation<ClaudeDesktopInstallResult>({
    mutationFn: async () => (await apiClient()).installClaudeDesktopExtension(),
  })
