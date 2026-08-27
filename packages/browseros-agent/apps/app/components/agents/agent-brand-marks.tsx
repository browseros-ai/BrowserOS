import type { FC } from 'react'
import HermesIcon from '@/assets/hermes.png'
import { Antigravity } from '@/components/ui/svgs/antigravity'
import { ClaudeAiIcon } from '@/components/ui/svgs/claudeAiIcon'
import { CodexDark } from '@/components/ui/svgs/codexDark'
import { CodexLight } from '@/components/ui/svgs/codexLight'
import { cn } from '@/lib/utils'

export interface AgentMarkProps {
  className?: string
}

/**
 * Real brand marks for agents, keyed by a brand id: the built-in agent types
 * (`claude`, `codex`) and the popular custom-agent ids (`opencode`, `openclaw`,
 * `antigravity`). Reuses the project's shared svg components where they exist.
 * Ids with no mark (pi, hermes) fall back to a monogram or a generic icon at the
 * call site.
 */

const ClaudeMark: FC<AgentMarkProps> = ({ className }) => (
  <ClaudeAiIcon className={className} />
)

const CodexMark: FC<AgentMarkProps> = ({ className }) => (
  <>
    <CodexLight className={cn(className, 'dark:hidden')} />
    <CodexDark className={cn('hidden dark:block', className)} />
  </>
)

const AntigravityMark: FC<AgentMarkProps> = ({ className }) => (
  <Antigravity className={className} />
)

// opencode's glyph. The shared svg carries a white background rect that reads
// as a box in tiles, so render just the glyph in opencode's brand ink (dark on
// light, white on dark) rather than the tile's accent color.
const OpencodeMark: FC<AgentMarkProps> = ({ className }) => (
  <svg
    viewBox="0 0 512 512"
    className={cn('text-[#17181c] dark:text-white', className)}
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    role="img"
    aria-label="opencode"
  >
    <path
      fillRule="evenodd"
      clipRule="evenodd"
      d="M384 416H128V96H384V416ZM320 160H192V352H320V160Z"
      fill="currentColor"
    />
  </svg>
)

const HermesMark: FC<AgentMarkProps> = ({ className }) => (
  <img
    src={HermesIcon}
    alt="Hermes"
    className={cn('h-full w-full object-contain', className)}
  />
)

const OpenClawMark: FC<AgentMarkProps> = ({ className }) => (
  <svg
    viewBox="0 0 120 120"
    className={className}
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    role="img"
    aria-label="OpenClaw"
  >
    <defs>
      <linearGradient id="openclaw-lobster" x1="0%" y1="0%" x2="100%" y2="100%">
        <stop offset="0%" stopColor="#ff4d4d" />
        <stop offset="100%" stopColor="#991b1b" />
      </linearGradient>
    </defs>
    <path
      d="M60 10 C30 10 15 35 15 55 C15 75 30 95 45 100 L45 110 L55 110 L55 100 C55 100 60 102 65 100 L65 110 L75 110 L75 100 C90 95 105 75 105 55 C105 35 90 10 60 10Z"
      fill="url(#openclaw-lobster)"
    />
    <path
      d="M20 45 C5 40 0 50 5 60 C10 70 20 65 25 55 C28 48 25 45 20 45Z"
      fill="url(#openclaw-lobster)"
    />
    <path
      d="M100 45 C115 40 120 50 115 60 C110 70 100 65 95 55 C92 48 95 45 100 45Z"
      fill="url(#openclaw-lobster)"
    />
    <path
      d="M45 15 Q35 5 30 8"
      stroke="#ff4d4d"
      strokeWidth="3"
      strokeLinecap="round"
    />
    <path
      d="M75 15 Q85 5 90 8"
      stroke="#ff4d4d"
      strokeWidth="3"
      strokeLinecap="round"
    />
    <circle cx="45" cy="35" r="6" fill="#050810" />
    <circle cx="75" cy="35" r="6" fill="#050810" />
    <circle cx="46" cy="34" r="2.5" fill="#00e5cc" />
    <circle cx="76" cy="34" r="2.5" fill="#00e5cc" />
  </svg>
)

const PiMark: FC<AgentMarkProps> = ({ className }) => (
  <svg
    viewBox="0 0 800 800"
    className={className}
    xmlns="http://www.w3.org/2000/svg"
    role="img"
    aria-label="pi"
  >
    <rect width="800" height="800" rx="120" fill="#09090b" />
    <path
      fill="#fff"
      fillRule="evenodd"
      d="M165.29 165.29H517.36V400H400V517.36H282.65V634.72H165.29ZM282.65 282.65V400H400V282.65Z"
    />
    <path fill="#fff" d="M517.36 400H634.72V634.72H517.36Z" />
  </svg>
)

/** Brand marks keyed by brand id (agent type or popular-agent id). */
export const BRAND_MARKS: Record<string, FC<AgentMarkProps>> = {
  claude: ClaudeMark,
  codex: CodexMark,
  opencode: OpencodeMark,
  openclaw: OpenClawMark,
  antigravity: AntigravityMark,
  pi: PiMark,
  hermes: HermesMark,
}

/**
 * The brand id to look up in {@link BRAND_MARKS} for an agent: its type for the
 * built-ins, or the popular-agent id stored on a custom agent (set when a user
 * picks a popular agent) for custom agents.
 */
export function agentBrandKey(agent: {
  type: string
  customConfig?: { icon?: string }
}): string | undefined {
  if (agent.type === 'custom') return agent.customConfig?.icon
  return agent.type
}
