import type { CustomAcpAgentConfig } from '@/modules/agents/acp-agent-types'

/** Parse a KEY=value textarea (one per line; blank lines and `#` comments skipped). */
export function parseEnvLines(text: string): Record<string, string> {
  const env: Record<string, string> = {}
  for (const raw of text.split('\n')) {
    const line = raw.trim()
    if (!line || line.startsWith('#')) continue
    const eq = line.indexOf('=')
    if (eq <= 0) continue
    const key = line.slice(0, eq).trim()
    if (key) env[key] = line.slice(eq + 1).trim()
  }
  return env
}

export function formatEnvLines(
  env: Record<string, string> | undefined,
): string {
  if (!env) return ''
  return Object.entries(env)
    .map(([key, value]) => `${key}=${value}`)
    .join('\n')
}

/** Split a comma-separated list into trimmed, non-empty entries. */
export function parseCsv(text: string): string[] {
  return text
    .split(',')
    .map((entry) => entry.trim())
    .filter(Boolean)
}

export interface CustomAgentFormState {
  command: string
  envText: string
  fullAccessModesText: string
  reasoningEffortKey: string
  systemPromptAppend: string
  icon: string
}

export function buildCustomConfig(
  state: CustomAgentFormState,
): CustomAcpAgentConfig {
  const env = parseEnvLines(state.envText)
  const fullAccessModes = parseCsv(state.fullAccessModesText)
  const reasoningEffortKey = state.reasoningEffortKey.trim()
  const systemPromptAppend = state.systemPromptAppend.trim()
  const icon = state.icon.trim()
  return {
    command: state.command.trim(),
    ...(Object.keys(env).length > 0 ? { env } : {}),
    ...(fullAccessModes.length > 0 ? { fullAccessModes } : {}),
    ...(reasoningEffortKey ? { reasoningEffortKey } : {}),
    ...(systemPromptAppend ? { systemPromptAppend } : {}),
    ...(icon ? { icon } : {}),
  }
}
