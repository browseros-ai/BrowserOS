/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * react-query-kit mutation backing the cockpit's Hand back control. Hits
 * `POST /api/v1/sessions/:sessionId/help/resolve`; the server signals the
 * waiting agent to resume with an optional note and reports whether a
 * pending request was found. A request that already timed out or was
 * cancelled comes back `{ resolved: false }`, which the caller surfaces.
 */

import type { ResolveHelpResponse } from '@browseros/claw-api'
import { createMutation } from 'react-query-kit'
import { apiClient } from './client'

export const useResolveHelp = createMutation<
  ResolveHelpResponse,
  { sessionId: string; note?: string }
>({
  mutationFn: async ({ sessionId, note }) =>
    (await apiClient()).resolveHelp({ sessionId, note }),
})
