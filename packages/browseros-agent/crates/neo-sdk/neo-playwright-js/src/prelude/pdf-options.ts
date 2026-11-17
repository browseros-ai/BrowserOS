/** The script API accepts numeric CSS pixels; the channel accepts strings, as upstream does. */
import { OPTION_KEYS } from './generated'
import { pick } from './options'
import type { PdfOptions } from './wire'

type Margin = NonNullable<PdfOptions['margin']>
type MarginInput = { [Side in keyof Margin]: Margin[Side] | number }
export type PdfOptionsInput = Omit<
  PdfOptions,
  'width' | 'height' | 'margin'
> & {
  width?: PdfOptions['width'] | number
  height?: PdfOptions['height'] | number
  margin?: MarginInput
}

function length(value: string | number | undefined): string | undefined {
  return typeof value === 'number' ? `${value}px` : value
}

function normalizeMargin(value: MarginInput | undefined): Margin | undefined {
  // Copy before reading sides: caller getters run once, and unknown keys reach Rust's
  // validator rather than disappearing during this client-side unit conversion.
  if (typeof value !== 'object' || value === null) return value
  const margin = { ...value }
  return {
    ...margin,
    top: length(margin.top),
    right: length(margin.right),
    bottom: length(margin.bottom),
    left: length(margin.left),
  }
}

export function normalizePdfOptions(options: PdfOptionsInput): PdfOptions {
  const wire = pick(options, OPTION_KEYS['page.pdf'])
  return {
    ...wire,
    width: length(wire.width),
    height: length(wire.height),
    margin: normalizeMargin(wire.margin),
  }
}
