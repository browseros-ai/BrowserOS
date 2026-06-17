/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * `navigate` tool. The first real tool wired through the permission
 * gate. Domain is parsed from the input URL, not from the agent's
 * site hint, because navigating is exactly the operation that
 * decides what site the run is on.
 */

import { z } from 'zod'
import type { ToolDefinition } from '../register'

const inputSchema = z.object({
  url: z.string().url(),
})

type Input = z.infer<typeof inputSchema>

export const navigateTool: ToolDefinition<Input> = {
  name: 'navigate',
  description: "Open a URL in the agent run's tab group.",
  verb: 'navigate',
  inputShape: { url: z.string().url() },
  parseInput: (raw) => inputSchema.parse(raw),
  domainFor: (input) => new URL(input.url).hostname,
  dispatch: (executor, run, input) =>
    executor.navigate(run, { url: input.url }),
}
