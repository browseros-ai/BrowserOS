/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { z } from 'zod'

export const ChatSourceSchema = z.enum(['sidepanel', 'newtab'])

export type ChatSource = z.infer<typeof ChatSourceSchema>
