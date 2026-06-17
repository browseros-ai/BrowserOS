/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * `click` tool. Maps to the `input` catalog verb (click & type, auto
 * by default). Site-rule clamping by domain still applies via the
 * run's site hint.
 */

import { z } from 'zod'
import type { ToolDefinition } from '../register'

const inputSchema = z.object({
  selector: z.string().min(1),
})

type Input = z.infer<typeof inputSchema>

export const clickTool: ToolDefinition<Input> = {
  name: 'click',
  description: 'Click an element by CSS selector.',
  verb: 'input',
  inputShape: { selector: z.string().min(1) },
  parseInput: (raw) => inputSchema.parse(raw),
  domainFor: (_input, run) => run.site,
  dispatch: (executor, run, input) =>
    executor.click(run, { selector: input.selector }),
}
