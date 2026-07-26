/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { z } from 'zod'

export const ConversationIdParamSchema = z.object({
  conversationId: z.string().uuid(),
})
