/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * react-query-kit mutation backing the cockpit's Stop button. Hits
 * `POST /api/v1/sessions/:sessionId/cancel`; the server terminally
 * cancels that BrowserClaw session and reports how many active browser
 * dispatches were interrupted. Missing or naturally ended sessions
 * come back 404/409, which the typed fetch client throws as the
 * mutation's error path.
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
