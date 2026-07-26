import { Bot } from 'lucide-react'
import type { FC } from 'react'
import type { Harness } from './harness.types'
import {
  AntigravityMark,
  ClaudeCodeMark,
  CodexMark,
  CursorMark,
  OpenCodeMark,
  VSCodeMark,
  ZedMark,
} from './harness-marks'

/**
 * Single icon component for any supported harness. Brand marks paint
 * themselves in their native colours, including light/dark variants,
 * so `className` is only used for sizing.
 */
export interface HarnessIconProps {
  harness: Harness
  className?: string
}

export const HarnessIcon: FC<HarnessIconProps> = ({ harness, className }) => {
  switch (harness) {
    case 'Claude Code':
      return <ClaudeCodeMark className={className} />
    case 'Codex':
      return <CodexMark className={className} />
    case 'Cursor':
      return <CursorMark className={className} />
    case 'OpenCode':
      return <OpenCodeMark className={className} />
    case 'Antigravity':
      return <AntigravityMark className={className} />
    case 'VS Code':
      return <VSCodeMark className={className} />
    case 'Zed':
      return <ZedMark className={className} />
    default: {
      // Exhaustiveness check: this line throws a TS error if a new
      // Harness is added to the union without a case above.
      const _exhaustive: never = harness
      void _exhaustive
      return <Bot className={className} aria-label="Harness" />
    }
  }
}
