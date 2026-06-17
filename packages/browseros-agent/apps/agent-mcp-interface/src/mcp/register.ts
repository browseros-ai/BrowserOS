/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Shared wrapping helper used by every real tool. Each tool exposes
 * a `ToolDefinition` describing its name + zod input shape + verb +
 * dispatch; the helper handles the boilerplate every tool needs:
 *
 *   1. parse the raw args into the typed input,
 *   2. extract the (domain, verb) pair the permission gate needs,
 *   3. call `permissions.check` and short-circuit on `block` / `ask`,
 *   4. start a stub run, dispatch, stop the run, return the
 *      observation as the MCP tool result.
 *
 * Phase 4 will replace the per-call run with a real run lifecycle.
 * For now, every tool call is its own ephemeral run, so the
 * deterministic stub executor stays usable for manual smoke + tests
 * without leaking handles.
 */

import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { ZodRawShape } from 'zod'
import { z } from 'zod'
import {
  type BrowserExecutor,
  ExecutorRunGoneError,
  type Observation,
  type RunHandle,
} from '../executor'
import type { StoredAgentProfile } from '../routes/agents/schemas'
import { check } from '../services/permissions'
import { asRegister, type ToolResult } from './register-fn'

export interface ToolDefinition<Input> {
  /** MCP tool name (what the harness calls). */
  readonly name: string
  readonly description: string
  /** Permission verb in the catalog space (`submit`, `payment`, etc). */
  readonly verb: string
  /** Zod shape passed to the SDK for its tools/list metadata. */
  readonly inputShape: ZodRawShape
  /** Parses + validates the raw args at the boundary. Throws on bad input. */
  parseInput(raw: Record<string, unknown>): Input
  /** How to derive the permission-check domain from this tool's input + run. */
  domainFor(input: Input, run: RunHandle): string
  /** Actually do the thing once permissions clear. */
  dispatch(
    executor: BrowserExecutor,
    run: RunHandle,
    input: Input,
  ): Promise<Observation>
}

const TASK_PLACEHOLDER = 'mcp tool call'

/**
 * Picks a domain for tools whose input doesn't carry one (every tool
 * except `navigate`). Phase 4's real run lifecycle will replace this
 * with the run's actual current URL; for now we use the agent's first
 * declared site as a stable hint, falling back to `'*'` so a user who
 * left `selectedSites` empty still gets covered by wildcard rules.
 */
export function domainFromAgent(agent: StoredAgentProfile): string {
  return agent.selectedSites[0] ?? '*'
}

export function registerTool<Input>(
  server: McpServer,
  agent: StoredAgentProfile,
  executor: BrowserExecutor,
  def: ToolDefinition<Input>,
): void {
  const register = asRegister(server)
  register(
    def.name,
    {
      description: def.description,
      inputSchema: def.inputShape,
    },
    async (rawArgs) => {
      const input = def.parseInput(rawArgs)

      const run = await executor.startRun({
        agentId: agent.id,
        task: TASK_PLACEHOLDER,
        site: domainFromAgent(agent),
      })
      try {
        const domain = def.domainFor(input, run)
        const verdict = await check({
          agentId: agent.id,
          verb: def.verb,
          domain,
        })
        if (verdict.verdict === 'block')
          return blockedResult(verdict.source, def.verb, domain)
        if (verdict.verdict === 'ask') return deferredResult(def.verb, domain)
        const observation = await def.dispatch(executor, run, input)
        return autoResult(observation)
      } catch (err) {
        if (err instanceof ExecutorRunGoneError) {
          return errorResult(`run has been stopped (${err.runId})`)
        }
        if (err instanceof z.ZodError) {
          return errorResult(`invalid input: ${err.message}`)
        }
        throw err
      } finally {
        await executor.stop(run)
      }
    },
  )
}

function autoResult(observation: Observation): ToolResult {
  return {
    content: [{ type: 'text', text: observation.summary }],
    structuredContent: { ...observation } as Record<string, unknown>,
  }
}

function blockedResult(
  source: string,
  verb: string,
  domain: string,
): ToolResult {
  return {
    content: [
      {
        type: 'text',
        text: `blocked by ${source}: ${verb} on ${domain}`,
      },
    ],
    isError: true,
  }
}

function deferredResult(verb: string, domain: string): ToolResult {
  return {
    content: [
      {
        type: 'text',
        text: `approval required for ${verb} on ${domain}; the cockpit will surface this once run-lifecycle approvals ship`,
      },
    ],
    isError: true,
  }
}

function errorResult(message: string): ToolResult {
  return {
    content: [{ type: 'text', text: message }],
    isError: true,
  }
}
