/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * `submit` tool. Maps to the `submit` catalog verb (Ask by default).
 * Payment-domain site rules take precedence over the agent's submit
 * verdict via the standard permissions.check precedence.
 */

import { z } from 'zod'
import type { ToolDefinition } from '../register'

const inputSchema = z.object({
  selector: z.string().min(1),
})

type Input = z.infer<typeof inputSchema>

export const submitTool: ToolDefinition<Input> = {
  name: 'submit',
  description: 'Submit the form containing the element at the CSS selector.',
  verb: 'submit',
  inputShape: { selector: z.string().min(1) },
  parseInput: (raw) => inputSchema.parse(raw),
  domainFor: (_input, run) => run.site,
  dispatch: (executor, run, input) =>
    executor.submit(run, { selector: input.selector }),
}
