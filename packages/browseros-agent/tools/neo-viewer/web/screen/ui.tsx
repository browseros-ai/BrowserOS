/** Small pieces the designed screen repeats: toned text, the two-way switch, dots and icons. */

import type { RunStatus } from '../../shared/api'
import type { Mark, MarkedSegment } from './find'
import type { Line, Tone } from './text-lines'

const TONE: Record<Tone, string> = {
  plain: '',
  muted: 'text-ink-3',
  key: 'text-ink-2',
  string: 'text-string',
  ref: 'text-neo',
  keyword: 'text-keyword',
  comment: 'text-ink-3',
  error: 'text-bad',
  note: 'text-busy-ink',
}

const MARK: Record<Mark, string> = {
  none: '',
  match: 'rounded-[2px] bg-[#FFF0B3]',
  current: 'rounded-[2px] bg-[#FFD54A]',
}

/** One line of toned text; with marks, search matches are highlighted in place. */
export function LineSpans({
  line,
  marked,
}: {
  line: Line
  marked?: MarkedSegment[]
}) {
  const pieces: MarkedSegment[] =
    marked ?? line.map((segment) => ({ ...segment, mark: 'none' }))
  return (
    <>
      {pieces.map((piece, index) => (
        <span
          // biome-ignore lint/suspicious/noArrayIndexKey: a line's pieces have no identity but their place.
          key={index}
          className={
            `${TONE[piece.tone]} ${MARK[piece.mark]}`.trim() || undefined
          }
          data-mark={piece.mark === 'none' ? undefined : piece.mark}
        >
          {piece.text}
        </span>
      ))}
    </>
  )
}

/** The design's two-way switch (Formatted | Raw); `size` follows the sidebar or the pop-up. */
export function Switch<T extends string>({
  options,
  value,
  onChange,
  size,
  label,
}: {
  options: readonly { value: T; label: string }[]
  value: T
  onChange: (value: T) => void
  size: 'small' | 'large'
  label: string
}) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={`flex min-w-0 overflow-clip border border-control ${size === 'small' ? 'rounded-[5px]' : 'rounded-md'}`}
    >
      {options.map((option) => {
        const active = option.value === value
        return (
          // biome-ignore lint/a11y/useSemanticElements: a segmented control, styled as the design's buttons.
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(option.value)}
            className={`${size === 'small' ? 'px-[9px] py-0.5 text-[11px] leading-[14px]' : 'px-2.5 py-1 text-xs leading-4'} ${active ? 'bg-neo-tint font-semibold text-neo' : 'text-ink hover:bg-panel'}`}
          >
            {option.label}
          </button>
        )
      })}
    </div>
  )
}

export const REPLY_VIEWS = [
  { value: 'formatted', label: 'Formatted' },
  { value: 'raw', label: 'Raw' },
] as const

export type ReplyView = (typeof REPLY_VIEWS)[number]['value']

const DOT: Record<RunStatus, string> = {
  running: 'bg-busy',
  ok: 'bg-ok',
  error: 'bg-bad',
  'rpc-error': 'bg-bad',
  'http-error': 'bg-bad',
  cancelled: 'bg-ink-4',
  lost: 'bg-ink-4',
}

export function StatusDot({ status }: { status: RunStatus }) {
  return (
    <span
      title={status}
      className={`block size-[7px] shrink-0 rounded-full ${DOT[status]}`}
    />
  )
}

/** The design's "open in a bigger view" mark, before "Open full" and "Pop out". */
export function OpenIcon() {
  return (
    <svg
      width="11"
      height="11"
      viewBox="0 0 11 11"
      aria-hidden="true"
      className="shrink-0"
    >
      <path
        d="M6.5 1.5h3v3M9.5 1.5L5.5 5.5M8 7v2.5H1.5V3H4"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
      />
    </svg>
  )
}

/** Token counts are estimates (chars ÷ 4); every one carries a quiet "≈". */
export function Approx() {
  return <span className="font-normal text-ink-4">≈</span>
}
