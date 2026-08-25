import type { FC } from 'react'

export interface AgentMarkProps {
  className?: string
}

/**
 * Brand marks for the Popular ACP agents list, sourced from svgl.app. opencode
 * is re-authored to a single `currentColor` glyph so it themes with the text
 * color; antigravity and openclaw are the full-color brand marks (ids namespaced
 * to avoid collisions). Agents without an svgl logo (pi, hermes) fall back to a
 * monogram in the dialog.
 */

const OpencodeMark: FC<AgentMarkProps> = ({ className }) => (
  <svg
    viewBox="0 0 512 512"
    className={className}
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

const AntigravityMark: FC<AgentMarkProps> = ({ className }) => (
  <svg
    viewBox="0 0 16 15"
    className={className}
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    role="img"
    aria-label="Antigravity"
  >
    <mask
      id="ag-mask"
      style={{ maskType: 'alpha' }}
      maskUnits="userSpaceOnUse"
      x="0"
      y="0"
      width="16"
      height="15"
    >
      <path
        d="M14.0777 13.984C14.945 14.6345 16.2458 14.2008 15.0533 13.0084C11.476 9.53949 12.2349 0 7.79033 0C3.34579 0 4.10461 9.53949 0.527295 13.0084C-0.773543 14.3092 0.635692 14.6345 1.50293 13.984C4.86344 11.7076 4.64663 7.69664 7.79033 7.69664C10.934 7.69664 10.7172 11.7076 14.0777 13.984Z"
        fill="black"
      />
    </mask>
    <g mask="url(#ag-mask)">
      <g filter="url(#ag-f0)">
        <path
          d="M-0.658907 -3.2306C-0.922679 -0.906781 1.07986 1.22861 3.81388 1.53894C6.54791 1.84927 8.97811 0.217009 9.24188 -2.10681C9.50565 -4.43063 7.50312 -6.56602 4.76909 -6.87635C2.03506 -7.18667 -0.395135 -5.55442 -0.658907 -3.2306Z"
          fill="#FFE432"
        />
      </g>
      <g filter="url(#ag-f1)">
        <path
          d="M9.88233 4.36642C10.5673 7.31568 13.566 9.13902 16.5801 8.43896C19.5942 7.73891 21.4823 4.78056 20.7973 1.83131C20.1123 -1.11795 17.1136 -2.94128 14.0995 -2.24123C11.0854 -1.54118 9.19733 1.41717 9.88233 4.36642Z"
          fill="#FC413D"
        />
      </g>
      <g filter="url(#ag-f2)">
        <path
          d="M-8.05291 6.34512C-7.18736 9.38883 -3.28925 10.9473 0.653774 9.82598C4.5968 8.7047 7.09158 5.32829 6.22603 2.28458C5.36048 -0.759142 1.46236 -2.31758 -2.48066 -1.19629C-6.42368 -0.0750048 -8.91846 3.3014 -8.05291 6.34512Z"
          fill="#00B95C"
        />
      </g>
      <g filter="url(#ag-f3)">
        <path
          d="M-4.92402 8.86746C-2.75421 11.0837 0.982691 10.9438 3.42257 8.55507C5.86246 6.1663 6.08139 2.43321 3.91158 0.216963C1.74177 -1.99928 -1.99513 -1.85942 -4.43501 0.529349C-6.87489 2.91812 -7.09383 6.65122 -4.92402 8.86746Z"
          fill="#00B95C"
        />
      </g>
      <g filter="url(#ag-f4)">
        <path
          d="M6.42819 17.2263C7.10197 20.1273 9.91278 21.953 12.7063 21.3042C15.4998 20.6553 17.2182 17.7777 16.5444 14.8767C15.8707 11.9757 13.0599 10.15 10.2663 10.7988C7.47281 11.4477 5.75441 14.3253 6.42819 17.2263Z"
          fill="#3186FF"
        />
      </g>
      <g filter="url(#ag-f5)">
        <path
          d="M1.66508 -5.94539C0.254213 -2.80254 1.7978 0.951609 5.11277 2.43973C8.42774 3.92785 12.2588 2.58642 13.6696 -0.556431C15.0805 -3.69928 13.5369 -7.45343 10.222 -8.94155C6.90699 -10.4297 3.07594 -9.08824 1.66508 -5.94539Z"
          fill="#FBBC04"
        />
      </g>
      <g filter="url(#ag-f6)">
        <path
          d="M18.5814 10.6598C17.6669 11.727 15.2806 11.1828 13.2514 9.44417C11.2222 7.70556 10.3185 5.43097 11.2329 4.3637C12.1473 3.29646 14.5336 3.84069 16.5628 5.57928C18.592 7.31789 19.4958 9.59249 18.5814 10.6598Z"
          fill="#749BFF"
        />
      </g>
      <g filter="url(#ag-f7)">
        <path
          d="M11.7552 5.22715C15.5162 7.77124 19.8471 7.93838 21.4286 5.60045C23.0101 3.26253 21.2433 -0.695128 17.4823 -3.23922C13.7213 -5.78331 9.39044 -5.95044 7.80896 -3.61252C6.22747 -1.27459 7.99428 2.68306 11.7552 5.22715Z"
          fill="#FC413D"
        />
      </g>
    </g>
    <defs>
      <filter id="ag-f0" x="-2" y="-8" width="14" height="12">
        <feGaussianBlur stdDeviation="0.72" />
      </filter>
      <filter id="ag-f1" x="2" y="-10" width="26" height="26">
        <feGaussianBlur stdDeviation="3.5" />
      </filter>
      <filter id="ag-f2" x="-15" y="-8" width="28" height="25">
        <feGaussianBlur stdDeviation="2.97" />
      </filter>
      <filter id="ag-f3" x="-13" y="-8" width="25" height="25">
        <feGaussianBlur stdDeviation="2.97" />
      </filter>
      <filter id="ag-f4" x="0" y="5" width="23" height="23">
        <feGaussianBlur stdDeviation="2.82" />
      </filter>
      <filter id="ag-f5" x="-4" y="-15" width="24" height="24">
        <feGaussianBlur stdDeviation="2.56" />
      </filter>
      <filter id="ag-f6" x="7" y="-1" width="17" height="16">
        <feGaussianBlur stdDeviation="2.04" />
      </filter>
      <filter id="ag-f7" x="4" y="-9" width="22" height="20">
        <feGaussianBlur stdDeviation="1.73" />
      </filter>
    </defs>
  </svg>
)

/** Brand marks keyed by popular-agent id. Absent ids fall back to a monogram. */
export const POPULAR_AGENT_MARKS: Record<string, FC<AgentMarkProps>> = {
  opencode: OpencodeMark,
  openclaw: OpenClawMark,
  antigravity: AntigravityMark,
}
