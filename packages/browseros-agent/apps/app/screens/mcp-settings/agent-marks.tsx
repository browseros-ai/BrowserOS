/**
 * Per-agent marks for the Integrations panel. Each mark is a thin
 * wrapper around the brand SVG installed from the SVGL shadcn
 * registry (see `apps/app/components.json`). The wrappers exist
 * so the consumer can pass a single `className` for sizing.
 *
 * The tile that hosts each mark is a flat white chip (in both
 * light and dark modes) so the brand's native colours render
 * correctly. Trying to recolour them via `currentColor` would
 * defeat the point of using brand-accurate artwork.
 */

import type { FC, SVGProps } from 'react'
import { AnthropicBlack } from '@/components/ui/svgs/anthropicBlack'
import { ClaudeAiIcon } from '@/components/ui/svgs/claudeAiIcon'
import { CodexLight } from '@/components/ui/svgs/codexLight'
import { CursorLight } from '@/components/ui/svgs/cursorLight'
import { Vscode } from '@/components/ui/svgs/vscode'
import { ZedLogo } from '@/components/ui/svgs/zedLogo'

export type AgentMarkProps = SVGProps<SVGSVGElement>

export const ClaudeMark: FC<AgentMarkProps> = (props) => (
  <AnthropicBlack aria-hidden {...props} />
)

// Kept so legacy installs that still have an active BrowserOS link
// to Claude Desktop render with the right brand mark; new users no
// longer see the row (filtered server-side in listAgents).
export const ClaudeDesktopMark: FC<AgentMarkProps> = (props) => (
  <ClaudeAiIcon aria-hidden {...props} />
)

export const CursorMark: FC<AgentMarkProps> = (props) => (
  <CursorLight aria-hidden {...props} />
)

export const VSCodeMark: FC<AgentMarkProps> = (props) => (
  <Vscode aria-hidden {...props} />
)

export const CodexMark: FC<AgentMarkProps> = (props) => (
  <CodexLight aria-hidden {...props} />
)

export const ZedMark: FC<AgentMarkProps> = (props) => (
  <ZedLogo aria-hidden {...props} />
)

export const GenericAgentMark: FC<AgentMarkProps> = ({
  className,
  ...rest
}) => (
  <svg
    viewBox="0 0 24 24"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    className={className}
    {...rest}
  >
    <circle
      cx="12"
      cy="12"
      r="9"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeDasharray="2.5 2"
    />
    <circle cx="12" cy="12" r="2.5" fill="currentColor" />
  </svg>
)
