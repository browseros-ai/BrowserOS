/**
 * Per-agent marks for the Integrations panel.
 *
 * Where the brand is carried by an existing project dependency
 * (`@lobehub/icons`), we use that — the same library the
 * `ProviderIcon` registry uses elsewhere in the settings UI. The
 * `Mono` variant renders the brand mark in `currentColor` so it
 * inherits the surrounding tile's tint without colour clashing.
 *
 * For agents not in `@lobehub/icons` (VS Code, Zed), we render
 * simple geometric primitives — schematic bracket pair for VS Code,
 * angled-Z stroke for Zed. Both use `aria-hidden` because the agent
 * name is always rendered alongside the glyph.
 */

import { Anthropic, Cursor, OpenAI } from '@lobehub/icons'
import type { FC } from 'react'

export interface AgentMarkProps {
  size?: number | string
  className?: string
}

type LobehubIcon = typeof Anthropic

const wrapLobehub = (Component: LobehubIcon): FC<AgentMarkProps> => {
  const Wrapped: FC<AgentMarkProps> = ({ size = 20, className }) => (
    <Component size={size} className={className} aria-hidden />
  )
  Wrapped.displayName = 'AgentMark(lobehub)'
  return Wrapped
}

export const ClaudeMark = wrapLobehub(Anthropic)
export const ClaudeDesktopMark = wrapLobehub(Anthropic)
export const CursorMark = wrapLobehub(Cursor)
export const CodexMark = wrapLobehub(OpenAI)

export const VSCodeMark: FC<AgentMarkProps> = ({ size = 20, className }) => (
  <svg
    viewBox="0 0 24 24"
    width={size}
    height={size}
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    className={className}
  >
    <path
      d="M9 6 L4 12 L9 18"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
    <path
      d="M15 6 L20 12 L15 18"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
    <path
      d="M14 5 L10 19"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
    />
  </svg>
)

export const ZedMark: FC<AgentMarkProps> = ({ size = 20, className }) => (
  <svg
    viewBox="0 0 24 24"
    width={size}
    height={size}
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    className={className}
  >
    <path
      d="M5.5 5.5 L18.5 5.5 L7 18.5 L18.5 18.5"
      stroke="currentColor"
      strokeWidth="2.4"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
)

export const GenericAgentMark: FC<AgentMarkProps> = ({
  size = 20,
  className,
}) => (
  <svg
    viewBox="0 0 24 24"
    width={size}
    height={size}
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    className={className}
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
