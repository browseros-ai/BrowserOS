/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  OpenClawCliProvider,
  OpenClawCliProviderAuthStatus,
} from './types'

const CLAUDE_CLI_MODELS = [
  'claude-sonnet-4-6',
  'claude-opus-4-6',
  'claude-haiku-4-5',
] as const

// `claude auth status` emits JSON on both the logged-in (exit 0) and
// not-logged-in (exit 1) paths. The caller passes us stdout alone —
// the exec layer separates stdout and stderr so no extraction or
// stripping of nerdctl noise is needed.
interface ClaudeAuthStatusPayload {
  loggedIn?: boolean
  email?: string
  subscriptionType?: string
}

function parseClaudeAuthStatus(
  stdout: string,
  exitCode: number,
): OpenClawCliProviderAuthStatus {
  const trimmed = stdout.trim()

  // Binary missing: claude isn't installed / not on PATH.
  if (exitCode === 127 || !trimmed) {
    return { installed: false, loggedIn: false }
  }

  let payload: ClaudeAuthStatusPayload
  try {
    payload = JSON.parse(trimmed) as ClaudeAuthStatusPayload
  } catch {
    return {
      installed: true,
      loggedIn: false,
      error: `Unexpected claude auth status output: ${trimmed.slice(0, 200)}`,
    }
  }

  return {
    installed: true,
    loggedIn: !!payload.loggedIn,
    accountLabel: payload.email,
    subscriptionLabel: payload.subscriptionType,
  }
}

export const CLAUDE_CLI_PROVIDER: OpenClawCliProvider = {
  id: 'claude-cli',
  displayName: 'Anthropic Claude CLI',
  description: 'Uses your Claude.ai subscription via the Claude Code CLI',
  npmPackage: '@anthropic-ai/claude-code',
  npmPackageVersion: '2.1.119',
  binary: 'claude',
  authStatusCommand: ['claude', 'auth', 'status'],
  // `claude auth login` in 2.1.x silently discards stdin. The REPL's
  // `/login` slash command, launched from a fresh `claude` invocation,
  // does accept a pasted token.
  authLoginCommand: 'claude /login',
  models: CLAUDE_CLI_MODELS,
  parseAuthStatus: parseClaudeAuthStatus,
}
