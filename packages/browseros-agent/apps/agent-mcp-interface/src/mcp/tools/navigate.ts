/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * `navigate` tool. The first real tool wired through the permission
 * gate. Domain is parsed from the input URL, not from the agent's
 * site hint, because navigating is exactly the operation that
 * decides what site the run is on.
 *
 * Only `http://` and `https://` URLs are accepted. `z.string().url()`
 * alone accepts any RFC-3986 URL (including `javascript:`, `file:`,
 * `data:`), and their `hostname` is the empty string, which would
 * leave the permission gate with no domain to match site rules
 * against. That would silently bypass the ACL once the real Chromium
 * executor lands; we reject those URIs at the MCP boundary instead.
 */

import { z } from 'zod'
import type { ToolDefinition } from '../register'

const httpUrlSchema = z
  .string()
  .url()
  .refine((value) => /^https?:\/\//i.test(value), {
    message: 'navigate accepts only http:// and https:// URLs',
  })

const inputSchema = z.object({
  url: httpUrlSchema,
})

type Input = z.infer<typeof inputSchema>

export const navigateTool: ToolDefinition<Input> = {
  name: 'navigate',
  description: "Open a URL in the agent run's tab group.",
  verb: 'navigate',
  inputShape: { url: httpUrlSchema },
  parseInput: (raw) => inputSchema.parse(raw),
  domainFor: (input) => new URL(input.url).hostname,
  dispatch: (executor, run, input) =>
    executor.navigate(run, { url: input.url }),
}
