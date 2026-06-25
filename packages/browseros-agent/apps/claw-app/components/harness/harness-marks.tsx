/**
 * Per-harness brand marks for the new-agent wizard and any other
 * surface that renders a harness tile. Each mark wraps a brand SVG
 * installed from the `@svgl` shadcn registry (see `components.json`).
 * Mirror of `apps/app/screens/mcp-settings/agent-marks.tsx`.
 *
 * Brand marks paint themselves with their own colours; do not pass
 * color via className. Tile chrome (bg, border) is the consumer's
 * job, the mark just needs sizing.
 */

import type { FC, SVGProps } from 'react'
import { AnthropicBlack } from '@/components/ui/svgs/anthropicBlack'
import { ClaudeAiIcon } from '@/components/ui/svgs/claudeAiIcon'
import { CodexLight } from '@/components/ui/svgs/codexLight'
import { CursorLight } from '@/components/ui/svgs/cursorLight'
import { Gemini } from '@/components/ui/svgs/gemini'
import { Vscode } from '@/components/ui/svgs/vscode'
import { ZedLogo } from '@/components/ui/svgs/zedLogo'

export type HarnessMarkProps = SVGProps<SVGSVGElement>

export const ClaudeCodeMark: FC<HarnessMarkProps> = (props) => (
  <AnthropicBlack aria-hidden {...props} />
)

export const ClaudeDesktopMark: FC<HarnessMarkProps> = (props) => (
  <ClaudeAiIcon aria-hidden {...props} />
)

export const CursorMark: FC<HarnessMarkProps> = (props) => (
  <CursorLight aria-hidden {...props} />
)

export const VSCodeMark: FC<HarnessMarkProps> = (props) => (
  <Vscode aria-hidden {...props} />
)

export const CodexMark: FC<HarnessMarkProps> = (props) => (
  <CodexLight aria-hidden {...props} />
)

export const ZedMark: FC<HarnessMarkProps> = (props) => (
  <ZedLogo aria-hidden {...props} />
)

export const GeminiMark: FC<HarnessMarkProps> = (props) => (
  <Gemini aria-hidden {...props} />
)
