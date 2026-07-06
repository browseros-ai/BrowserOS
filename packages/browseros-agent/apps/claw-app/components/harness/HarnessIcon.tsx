import { Bot, PawPrint, Wand2 } from 'lucide-react'
import type { FC } from 'react'
import type { Harness } from './harness.types'
import {
  ClaudeCodeMark,
  ClaudeDesktopMark,
  CodexMark,
  CursorMark,
  GeminiMark,
  VSCodeMark,
  ZedMark,
} from './harness-marks'

/**
 * Single icon component for any supported harness. External
 * harnesses render brand SVG marks; BrowserOS-internal harnesses
 * fall through to lucide picks because they have no third-party
 * brand identity.
 *
 * Brand marks paint themselves in their native colours, so
 * `className` is only used for sizing. Lucide picks accept the
 * full Tailwind className (text-color, etc.).
 */
export interface HarnessIconProps {
  harness: Harness
  className?: string
}

export const HarnessIcon: FC<HarnessIconProps> = ({ harness, className }) => {
  switch (harness) {
    case 'Claude Code':
      return <ClaudeCodeMark className={className} />
    case 'Claude Desktop':
      return <ClaudeDesktopMark className={className} />
    case 'Cursor':
      return <CursorMark className={className} />
    case 'VS Code':
      return <VSCodeMark className={className} />
    case 'Zed':
      return <ZedMark className={className} />
    case 'Codex':
      return <CodexMark className={className} />
    case 'Gemini CLI':
      return <GeminiMark className={className} />
    case 'Hermes':
      return <Wand2 className={className} aria-label="Hermes" />
    case 'OpenClaw':
      return <PawPrint className={className} aria-label="OpenClaw" />
    default: {
      // Exhaustiveness check: this line throws a TS error if a new
      // Harness is added to the union without a case above.
      const _exhaustive: never = harness
      void _exhaustive
      return <Bot className={className} aria-label="Harness" />
    }
  }
}
