/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * `read` tool. Reads a DOM observation from the current tab. Maps to
 * the `input` catalog verb (low-risk, auto by default) because the
 * roadmap deliberately keeps reading out of the gated action list;
 * the agent is allowed to look. Domain is the agent's site hint since
 * a read doesn't carry a URL.
 */

import { z } from 'zod'
import type { ToolDefinition } from '../register'

const inputSchema = z.object({
  selector: z.string().optional(),
})

type Input = z.infer<typeof inputSchema>

export const readTool: ToolDefinition<Input> = {
  name: 'read',
  description:
    'Read a DOM observation from the current tab. Optional CSS selector narrows the read.',
  verb: 'input',
  inputShape: { selector: z.string().optional() },
  parseInput: (raw) => inputSchema.parse(raw),
  // The run handle carries the agent's site hint set at startRun
  // (Phase 4 swaps it for the live URL).
  domainFor: (_input, run) => run.site,
  dispatch: (executor, run, input) =>
    executor.read(run, { selector: input.selector }),
}
