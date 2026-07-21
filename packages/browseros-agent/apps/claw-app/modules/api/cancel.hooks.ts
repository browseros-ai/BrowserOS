/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * react-query-kit mutation backing the cockpit's Stop button. Hits
 * `POST /api/v1/sessions/:sessionId/cancel`; the server aborts every
 * in-flight tool dispatch on that MCP session and resolves with the
 * count — `cancelled: 0` is a success meaning the session was idle.
 * Missing or already-ended sessions come back 404/409, which the
 * typed fetch client throws as the mutation's error path. The session
 * itself stays open: the agent's harness sees each cancelled dispatch
 * as an isError tool result and is free to fire its next call.
 */

import type { CancelSessionResponse } from '@browseros/claw-api'
import { createMutation } from 'react-query-kit'
import { apiClient } from './client'

export const useCancelSession = createMutation<
  CancelSessionResponse,
  { sessionId: string }
>({
  mutationFn: async ({ sessionId }) =>
    (await apiClient()).cancelSession({ sessionId }),
})
