/**
 * Per-agent SVG marks. Schematic geometric forms — not reproductions
 * of trademarked logos — so each row in the Integrations list is
 * instantly recognisable without putting a vendor's mark on screen.
 * Every glyph uses `currentColor` so the surrounding tile decides
 * the hue.
 */

import type { FC, SVGProps } from 'react'

type MarkProps = SVGProps<SVGSVGElement>

export const ClaudeMark: FC<MarkProps> = (props) => (
  <svg
    viewBox="0 0 24 24"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    {...props}
  >
    <path
      d="M12 1.5 L13.7 9.7 L21.9 11.4 L13.7 13.1 L12 21.3 L10.3 13.1 L2.1 11.4 L10.3 9.7 Z"
      fill="currentColor"
    />
  </svg>
)

export const ClaudeDesktopMark: FC<MarkProps> = (props) => (
  <svg
    viewBox="0 0 24 24"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    {...props}
  >
    <rect
      x="2.75"
      y="4.75"
      width="18.5"
      height="14.5"
      rx="2.25"
      stroke="currentColor"
      strokeWidth="1.5"
    />
    <circle cx="6" cy="8" r="0.75" fill="currentColor" />
    <circle cx="8.5" cy="8" r="0.75" fill="currentColor" />
    <path
      d="M12 10.5 L12.85 13.15 L15.5 14 L12.85 14.85 L12 17.5 L11.15 14.85 L8.5 14 L11.15 13.15 Z"
      fill="currentColor"
    />
  </svg>
)

export const CursorMark: FC<MarkProps> = (props) => (
  <svg
    viewBox="0 0 24 24"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    {...props}
  >
    <path d="M4 2.5 L19 10.5 L12.5 12.3 L10.7 19 Z" fill="currentColor" />
  </svg>
)

export const VSCodeMark: FC<MarkProps> = (props) => (
  <svg
    viewBox="0 0 24 24"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    {...props}
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

export const CodexMark: FC<MarkProps> = (props) => (
  <svg
    viewBox="0 0 24 24"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    {...props}
  >
    <path
      d="M12 2.5 L20.5 7.25 L20.5 16.75 L12 21.5 L3.5 16.75 L3.5 7.25 Z"
      stroke="currentColor"
      strokeWidth="1.6"
    />
    <circle cx="12" cy="12" r="2.5" fill="currentColor" />
  </svg>
)

export const ZedMark: FC<MarkProps> = (props) => (
  <svg
    viewBox="0 0 24 24"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    {...props}
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

export const GenericAgentMark: FC<MarkProps> = (props) => (
  <svg
    viewBox="0 0 24 24"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    {...props}
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
