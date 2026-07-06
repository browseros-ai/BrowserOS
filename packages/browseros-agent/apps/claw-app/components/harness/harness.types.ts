export const HARNESSES = [
  'Claude Code',
  'Claude Desktop',
  'Cursor',
  'VS Code',
  'Zed',
  'Codex',
  'Gemini CLI',
  'Hermes',
  'OpenClaw',
] as const

export type Harness = (typeof HARNESSES)[number]

export const RETIRED_HARNESSES = [
  'Claude Desktop',
] as const satisfies readonly Harness[]
