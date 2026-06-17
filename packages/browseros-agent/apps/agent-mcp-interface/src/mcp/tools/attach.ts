/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * `attach` tool. Maps to the `upload` catalog verb (Ask by default).
 *
 * Path safety: the durable file-access gate is the vault that lands
 * with the Chrome import work; until then we apply defense in depth
 * at the MCP boundary by rejecting `..` path segments so a
 * prompt-injected harness cannot smuggle a traversal-shaped path
 * through to the executor. Absolute paths are still permitted (a
 * user attaching `/Users/me/Downloads/receipt.pdf` is the expected
 * shape) but the vault will narrow this further with an allowlist of
 * roots the agent is permitted to read from.
 */

import { z } from 'zod'
import type { ToolDefinition } from '../register'

const filePathSchema = z
  .string()
  .min(1)
  .refine((value) => !value.split(/[\\/]/).includes('..'), {
    message: 'filePath must not contain ".." segments',
  })

const inputSchema = z.object({
  selector: z.string().min(1),
  filePath: filePathSchema,
})

type Input = z.infer<typeof inputSchema>

export const attachTool: ToolDefinition<Input> = {
  name: 'attach',
  description: 'Attach a local file to a file input by CSS selector.',
  verb: 'upload',
  inputShape: { selector: z.string().min(1), filePath: filePathSchema },
  parseInput: (raw) => inputSchema.parse(raw),
  domainFor: (_input, run) => run.site,
  dispatch: (executor, run, input) =>
    executor.attach(run, {
      selector: input.selector,
      filePath: input.filePath,
    }),
}
