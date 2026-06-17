/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * `type` tool. Maps to the `input` catalog verb.
 */

import { z } from 'zod'
import type { ToolDefinition } from '../register'

const inputSchema = z.object({
  selector: z.string().min(1),
  value: z.string(),
})

type Input = z.infer<typeof inputSchema>

export const typeTool: ToolDefinition<Input> = {
  name: 'type',
  description: 'Type a value into an input by CSS selector.',
  verb: 'input',
  inputShape: { selector: z.string().min(1), value: z.string() },
  parseInput: (raw) => inputSchema.parse(raw),
  domainFor: (_input, run) => run.site,
  dispatch: (executor, run, input) =>
    executor.type(run, { selector: input.selector, value: input.value }),
}
