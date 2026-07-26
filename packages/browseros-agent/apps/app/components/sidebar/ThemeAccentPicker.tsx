import type { FC, PointerEvent as ReactPointerEvent } from 'react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { accentThemeStorage } from '@/lib/theme/accent-theme-storage'
import { hexToHsv, hsvToRgb, rgbToHex } from '@/lib/theme/accentPalette'
import { mountAccentThemeCss } from '@/lib/theme/accentThemeDocument'
import { cn } from '@/lib/utils'

function clamp(n: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, n))
}

const DEFAULT_HEX = '#f3e4c9'

/** Same row layout as Shortcuts / About in the sidebar footer */
const SIDEBAR_FOOTER_ROW =
  'flex h-9 w-full items-center justify-start gap-2 overflow-hidden whitespace-nowrap rounded-md px-3 font-medium text-sm transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground'

function parseUserHex(raw: string): string | null {
  const t = raw.trim()
  const withHash = t.startsWith('#') ? t : `#${t}`
  if (!/^#[0-9a-fA-F]{6}$/.test(withHash)) return null
  return withHash.toLowerCase()
}

const ThemeColorPickerPanel: FC<{
  initialHex: string
  onApply: (hex: string) => void | Promise<void>
  onReset: () => void | Promise<void>
  onClose: () => void
  onFinished?: () => void
}> = ({ initialHex, onApply, onReset, onClose, onFinished }) => {
  const start = useMemo(() => {
    const hsv = hexToHsv(initialHex)
    return { ...hsv, hex: initialHex }
  }, [initialHex])

  const [hue, setHue] = useState(start.h)
  const [sat, setSat] = useState(start.s)
  const [val, setVal] = useState(start.v)
  const [hexInput, setHexInput] = useState(start.hex)

  useEffect(() => {
    setHue(start.h)
    setSat(start.s)
    setVal(start.v)
    setHexInput(start.hex)
  }, [start.h, start.s, start.v, start.hex])

  const canvasRef = useRef<HTMLCanvasElement>(null)
  const draggingSv = useRef(false)

  const pushHexFromHsv = useCallback((h: number, s: number, v: number) => {
    const { r, g, b } = hsvToRgb(h, s, v)
    setHexInput(rgbToHex(r, g, b))
  }, [])

  const drawSvPlane = useCallback(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return
    const w = canvas.width
    const h = canvas.height
    const img = ctx.createImageData(w, h)
    const data = img.data
    for (let y = 0; y < h; y++) {
      const vv = 1 - y / (h - 1 || 1)
      for (let x = 0; x < w; x++) {
        const ss = x / (w - 1 || 1)
        const { r, g, b } = hsvToRgb(hue, ss, vv)
        const i = (y * w + x) * 4
        data[i] = r
        data[i + 1] = g
        data[i + 2] = b
        data[i + 3] = 255
      }
    }
    ctx.putImageData(img, 0, 0)
  }, [hue])

  useEffect(() => {
    drawSvPlane()
  }, [drawSvPlane])

  const readSvFromPointer = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current
    if (!canvas) return
    const r = canvas.getBoundingClientRect()
    const x = clamp(e.clientX - r.left, 0, r.width)
    const y = clamp(e.clientY - r.top, 0, r.height)
    const s = x / r.width
    const v = 1 - y / r.height
    setSat(s)
    setVal(v)
    pushHexFromHsv(hue, s, v)
  }

  const onSvPointerDown = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    draggingSv.current = true
    e.currentTarget.setPointerCapture(e.pointerId)
    readSvFromPointer(e)
  }

  const onSvPointerMove = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    if (!draggingSv.current) return
    readSvFromPointer(e)
  }

  const onSvPointerUp = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    draggingSv.current = false
    try {
      e.currentTarget.releasePointerCapture(e.pointerId)
    } catch {
      // ignore
    }
  }

  const applyHexInput = () => {
    const hex = parseUserHex(hexInput)
    if (!hex) return
    const { h, s, v } = hexToHsv(hex)
    setHue(h)
    setSat(s)
    setVal(v)
    setHexInput(hex)
  }

  const previewHex = useMemo(() => {
    const { r, g, b } = hsvToRgb(hue, sat, val)
    return rgbToHex(r, g, b)
  }, [hue, sat, val])

  return (
    <div className="flex flex-col gap-3.5">
      <canvas
        ref={canvasRef}
        width={260}
        height={152}
        className="w-full cursor-crosshair touch-none rounded-xl border border-border/80 bg-muted/20 shadow-[inset_0_0_0_1px_rgba(0,0,0,0.04)]"
        onPointerDown={onSvPointerDown}
        onPointerMove={onSvPointerMove}
        onPointerUp={onSvPointerUp}
        onPointerLeave={onSvPointerUp}
        aria-label="Saturation and brightness"
      />
      <div
        className="relative h-2.5 w-full overflow-hidden rounded-full border border-border/70 shadow-inner"
        style={{
          background:
            'linear-gradient(to right,#f87171,#fbbf24,#4ade80,#22d3ee,#60a5fa,#c084fc,#f472b6,#f87171)',
        }}
      >
        <input
          type="range"
          min={0}
          max={360}
          step={1}
          value={Math.round(hue)}
          onChange={(e) => {
            const nh = Number(e.target.value)
            setHue(nh)
            pushHexFromHsv(nh, sat, val)
          }}
          className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
          aria-label="Hue"
        />
        <div
          className="pointer-events-none absolute top-1/2 h-3.5 w-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-background bg-background/90 shadow-sm ring-1 ring-foreground/15"
          style={{ left: `${(hue / 360) * 100}%` }}
        />
      </div>
      <div className="flex items-center gap-2.5">
        <div
          className="size-8 shrink-0 rounded-full border border-border/90 shadow-sm ring-1 ring-black/[0.06]"
          style={{ backgroundColor: previewHex }}
        />
        <Input
          value={hexInput}
          onChange={(e) => setHexInput(e.target.value)}
          onBlur={applyHexInput}
          onKeyDown={(e) => {
            if (e.key === 'Enter') applyHexInput()
          }}
          className="h-9 flex-1 rounded-lg border-border/90 bg-background/70 font-mono text-foreground text-xs shadow-sm"
          spellCheck={false}
        />
      </div>
      <div className="flex justify-end gap-2 border-border/50 border-t pt-3">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="rounded-full px-4 text-muted-foreground hover:bg-muted/80 hover:text-foreground"
          onClick={async () => {
            await Promise.resolve(onReset())
            onClose()
            onFinished?.()
          }}
        >
          Reset
        </Button>
        <Button
          type="button"
          size="sm"
          className="rounded-full bg-primary px-5 text-primary-foreground shadow-sm hover:bg-primary/90"
          onClick={async () => {
            await Promise.resolve(onApply(previewHex))
            onClose()
            onFinished?.()
          }}
        >
          Apply
        </Button>
      </div>
    </div>
  )
}

interface ThemeAccentPickerProps {
  expanded?: boolean
  /** Collapse expanded sidebar / mobile sheet after Apply or Reset */
  onApplied?: () => void
}

/**
 * Sidebar control: opens a color palette to set a custom theme (persisted, applies app-wide).
 * @public
 */
export const ThemeAccentPicker: FC<ThemeAccentPickerProps> = ({
  expanded = true,
  onApplied,
}) => {
  const [open, setOpen] = useState(false)
  const [savedHex, setSavedHex] = useState<string | null>(null)

  useEffect(() => {
    accentThemeStorage.getValue().then(setSavedHex)
    return accentThemeStorage.watch((v) => setSavedHex(v))
  }, [])

  const ballColor = savedHex ?? DEFAULT_HEX

  const triggerButton = (
    <button
      type="button"
      aria-label="Theme color"
      title={expanded ? undefined : 'Theme color'}
      className={cn(SIDEBAR_FOOTER_ROW)}
    >
      <span
        className="size-4 shrink-0 rounded-full border border-sidebar-border/80 shadow-sm ring-1 ring-black/[0.06]"
        style={{ backgroundColor: ballColor }}
      />
      <span
        className={cn(
          'truncate transition-opacity duration-200',
          expanded ? 'opacity-100' : 'opacity-0',
        )}
      >
        Theme color
      </span>
    </button>
  )

  const body = (
    <>
      <p className="mb-1 font-medium text-foreground text-xs tracking-wide opacity-80">
        Theme
      </p>
      <ThemeColorPickerPanel
        key={open ? 'picker-open' : 'picker-shut'}
        initialHex={savedHex ?? DEFAULT_HEX}
        onApply={async (hex) => {
          await accentThemeStorage.setValue(hex)
          mountAccentThemeCss(hex)
          setSavedHex(hex)
        }}
        onReset={async () => {
          await accentThemeStorage.setValue(null)
          mountAccentThemeCss(null)
          setSavedHex(null)
        }}
        onClose={() => setOpen(false)}
        onFinished={onApplied}
      />
    </>
  )

  const popover = (
    <Popover open={open} onOpenChange={setOpen}>
      {!expanded ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <PopoverTrigger asChild>{triggerButton}</PopoverTrigger>
          </TooltipTrigger>
          <TooltipContent side="right" className="text-xs">
            Theme color
          </TooltipContent>
        </Tooltip>
      ) : (
        <PopoverTrigger asChild>{triggerButton}</PopoverTrigger>
      )}
      <PopoverContent
        side="right"
        align="start"
        className={cn(
          'w-[min(92vw,18.5rem)] border border-border/80 bg-card/95 p-4 text-card-foreground shadow-lg',
          'rounded-2xl backdrop-blur-md',
        )}
        collisionPadding={12}
        onCloseAutoFocus={(e) => e.preventDefault()}
      >
        {body}
      </PopoverContent>
    </Popover>
  )

  if (!expanded) {
    return <TooltipProvider delayDuration={0}>{popover}</TooltipProvider>
  }

  return popover
}
