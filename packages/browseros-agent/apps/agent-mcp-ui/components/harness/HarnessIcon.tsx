import {
  Bot,
  Code,
  Code2,
  Cpu,
  Gem,
  MousePointer2,
  PawPrint,
  Sparkles,
  Terminal,
  Wand2,
  Zap,
} from 'lucide-react'
import type { FC } from 'react'
import type { Harness } from '@/screens/new-agent/new-agent.schemas'

/**
 * Single icon component for any harness the wizard can target. Same
 * shape as apps/agent's AdapterIcon: typed switch, aria-label per
 * harness, generic Bot fallback so future harnesses land without
 * a code change at the call site.
 *
 * These are lucide-react picks chosen to read close to brand
 * identity (Code2 for VS Code, Zap for Zed, Cpu for Codex, etc.).
 * Vendor in real brand SVGs as a follow-up if needed; this file is
 * the only swap point.
 */
export interface HarnessIconProps {
  harness: Harness
  className?: string
}

export const HarnessIcon: FC<HarnessIconProps> = ({ harness, className }) => {
  switch (harness) {
    case 'Claude Code':
      return <Terminal className={className} aria-label="Claude Code" />
    case 'Claude Desktop':
      return <Sparkles className={className} aria-label="Claude Desktop" />
    case 'Cursor':
      return <MousePointer2 className={className} aria-label="Cursor" />
    case 'VS Code':
      return <Code2 className={className} aria-label="VS Code" />
    case 'Zed':
      return <Zap className={className} aria-label="Zed" />
    case 'Codex':
      return <Code className={className} aria-label="Codex" />
    case 'Gemini CLI':
      return <Gem className={className} aria-label="Gemini CLI" />
    case 'Hermes':
      return <Wand2 className={className} aria-label="Hermes" />
    case 'OpenClaw':
      return <PawPrint className={className} aria-label="OpenClaw" />
    default: {
      // Exhaustiveness check: this line throws a TS error if a new
      // Harness is added to the union without a case above.
      const _exhaustive: never = harness
      void _exhaustive
      void Cpu
      return <Bot className={className} aria-label="Harness" />
    }
  }
}

/**
 * True when the harness corresponds to a third-party CLI/IDE we will
 * write a real MCP config entry for. False for BrowserOS-internal
 * harnesses (Hermes, OpenClaw). Useful for UI affordances that want
 * to render the BrowserOS-internal group differently (e.g. a "runs
 * inside BrowserOS" subtitle).
 */
export function isExternalHarness(harness: Harness): boolean {
  return harness !== 'Hermes' && harness !== 'OpenClaw'
}
