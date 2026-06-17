/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * `attach` tool. Maps to the `upload` catalog verb (Ask by default).
 * The filesystem path stays out of the permission check; file access
 * controls are a Phase 8 (vault) concern.
 */

import { z } from 'zod'
import type { ToolDefinition } from '../register'

const inputSchema = z.object({
  selector: z.string().min(1),
  filePath: z.string().min(1),
})

type Input = z.infer<typeof inputSchema>

export const attachTool: ToolDefinition<Input> = {
  name: 'attach',
  description: 'Attach a local file to a file input by CSS selector.',
  verb: 'upload',
  inputShape: { selector: z.string().min(1), filePath: z.string().min(1) },
  parseInput: (raw) => inputSchema.parse(raw),
  domainFor: (_input, run) => run.site,
  dispatch: (executor, run, input) =>
    executor.attach(run, {
      selector: input.selector,
      filePath: input.filePath,
    }),
}
