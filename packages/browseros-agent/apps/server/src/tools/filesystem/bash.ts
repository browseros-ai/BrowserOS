import { resolve } from 'node:path'
import { type Tool, tool } from 'ai'
import { z } from 'zod'
import {
  DEFAULT_BASH_TIMEOUT,
  executeWithMetrics,
  toModelOutput,
  truncateTail,
} from './utils'

const TOOL_NAME = 'filesystem_bash'

function getShellArgs(): [string, string] {
  if (process.platform === 'win32') return ['cmd.exe', '/c']
  return [process.env.SHELL || '/bin/sh', '-c']
}

export function createBashTool(cwd: string): Tool {
  return tool({
    description:
      'Execute a shell command and return its output. Commands run in a shell (sh/bash on Unix, cmd on Windows). Output is truncated to the last 2000 lines if too large.',
    inputSchema: z.object({
      command: z.string().describe('Shell command to execute'),
      timeout: z
        .number()
        .optional()
        .describe(`Timeout in seconds (default: ${DEFAULT_BASH_TIMEOUT})`),
    }),
    execute: (params) =>
      executeWithMetrics(TOOL_NAME, async () => {
        const [shell, flag] = getShellArgs()
        const timeoutMs = (params.timeout || DEFAULT_BASH_TIMEOUT) * 1000
        const resolvedCwd = resolve(cwd)

        const proc = Bun.spawn([shell, flag, params.command], {
          cwd: resolvedCwd,
          stdout: 'pipe',
          stderr: 'pipe',
          env: { ...process.env },
        })

        let timedOut = false
        const timer = setTimeout(() => {
          timedOut = true
          proc.kill()
        }, timeoutMs)

        const [stdoutText, stderrText] = await Promise.all([
          new Response(proc.stdout).text(),
          new Response(proc.stderr).text(),
        ])

        const exitCode = await proc.exited
        clearTimeout(timer)

        if (timedOut) {
          let output = stdoutText
          if (stderrText) output += (output ? '\n' : '') + stderrText
          const truncated = truncateTail(output)
          return {
            text: `Command timed out after ${params.timeout || DEFAULT_BASH_TIMEOUT}s\n\n${truncated.content}`,
            isError: true,
          }
        }

        let output = stdoutText
        if (stderrText) output += (output ? '\n' : '') + stderrText

        const truncated = truncateTail(output)
        let result = truncated.content
        if (truncated.truncated) {
          result = `(Output truncated. Showing last ${truncated.keptLines} of ${truncated.totalLines} lines)\n${result}`
        }

        if (exitCode !== 0) {
          result += `\n\n[Exit code: ${exitCode}]`
          return { text: result, isError: true }
        }

        return { text: result || '(no output)' }
      }),
    toModelOutput,
  })
}
