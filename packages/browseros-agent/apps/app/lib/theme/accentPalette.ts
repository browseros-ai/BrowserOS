/**
 * Derive full theme CSS variables from a single hex accent (user-chosen color).
 */

export function hexToRgb(
  hex: string,
): { r: number; g: number; b: number } | null {
  const h = hex.trim().replace(/^#/, '')
  if (h.length !== 6 || !/^[0-9a-fA-F]+$/.test(h)) return null
  return {
    r: Number.parseInt(h.slice(0, 2), 16),
    g: Number.parseInt(h.slice(2, 4), 16),
    b: Number.parseInt(h.slice(4, 6), 16),
  }
}

export function rgbToHsl(
  r: number,
  g: number,
  b: number,
): { h: number; s: number; l: number } {
  const rn = r / 255
  const gn = g / 255
  const bn = b / 255
  const max = Math.max(rn, gn, bn)
  const min = Math.min(rn, gn, bn)
  const d = max - min
  let h = 0
  const l = (max + min) / 2
  let s = 0

  if (d > 1e-6) {
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
    switch (max) {
      case rn:
        h = ((gn - bn) / d + (gn < bn ? 6 : 0)) / 6
        break
      case gn:
        h = ((bn - rn) / d + 2) / 6
        break
      default:
        h = ((rn - gn) / d + 4) / 6
        break
    }
  }

  return { h: h * 360, s: s * 100, l: l * 100 }
}

export function hslCss(h: number, s: number, l: number): string {
  return `hsl(${Math.round(h)} ${Math.max(0, Math.min(100, Math.round(s)))}% ${Math.max(0, Math.min(100, Math.round(l)))}%)`
}

/** HSV: h 0–360, s/v 0–1 */
export function hsvToRgb(
  h: number,
  s: number,
  v: number,
): { r: number; g: number; b: number } {
  const hh = ((h % 360) + 360) % 360
  const c = v * s
  const x = c * (1 - Math.abs(((hh / 60) % 2) - 1))
  const m = v - c
  let rp = 0
  let gp = 0
  let bp = 0
  if (hh < 60) {
    rp = c
    gp = x
  } else if (hh < 120) {
    rp = x
    gp = c
  } else if (hh < 180) {
    gp = c
    bp = x
  } else if (hh < 240) {
    gp = x
    bp = c
  } else if (hh < 300) {
    rp = x
    bp = c
  } else {
    rp = c
    bp = x
  }
  return {
    r: Math.round((rp + m) * 255),
    g: Math.round((gp + m) * 255),
    b: Math.round((bp + m) * 255),
  }
}

export function rgbToHex(r: number, g: number, b: number): string {
  const to = (n: number) =>
    Math.max(0, Math.min(255, Math.round(n)))
      .toString(16)
      .padStart(2, '0')
  return `#${to(r)}${to(g)}${to(b)}`.toLowerCase()
}

export function hexToHsv(hex: string): { h: number; s: number; v: number } {
  const rgb = hexToRgb(hex)
  if (!rgb) return { h: 0, s: 1, v: 1 }
  const r = rgb.r / 255
  const g = rgb.g / 255
  const b = rgb.b / 255
  const max = Math.max(r, g, b)
  const min = Math.min(r, g, b)
  const d = max - min
  let h = 0
  const s = max === 0 ? 0 : d / max
  const v = max

  if (d > 1e-6) {
    if (max === r) h = 60 * (((g - b) / d + 6) % 6)
    else if (max === g) h = 60 * ((b - r) / d + 2)
    else h = 60 * ((r - g) / d + 4)
  }

  return { h, s, v }
}

function chartHue(h: number, offset: number): string {
  return hslCss((h + offset + 360) % 360, 48, 55)
}

function buildLight(h: number, s: number): Record<string, string> {
  const sat = Math.max(s, 14)
  return {
    '--background': hslCss(h, Math.min(sat * 0.42 + 8, 32), 92),
    '--foreground': hslCss(h, 22, 14),
    '--card': hslCss(h, Math.min(sat * 0.32 + 6, 26), 97),
    '--card-foreground': hslCss(h, 22, 14),
    '--popover': hslCss(h, Math.min(sat * 0.32 + 6, 26), 97),
    '--popover-foreground': hslCss(h, 22, 14),
    '--primary': hslCss(h, Math.min(sat + 28, 82), 34),
    '--primary-foreground': hslCss(h, 12, 98),
    '--secondary': hslCss(h, Math.min(sat * 0.28 + 5, 22), 90),
    '--secondary-foreground': hslCss(h, 22, 14),
    '--muted': hslCss(h, Math.min(sat * 0.3 + 6, 24), 88),
    '--muted-foreground': hslCss(h, 14, 42),
    '--accent': hslCss(h, Math.min(sat * 0.28 + 6, 24), 86),
    '--accent-foreground': hslCss(h, 22, 14),
    '--border': hslCss(h, Math.min(sat * 0.22 + 5, 20), 78),
    '--input': hslCss(h, Math.min(sat * 0.22 + 5, 20), 78),
    '--ring': hslCss(h, Math.min(sat + 15, 55), 52),
    '--sidebar': hslCss(h, Math.min(sat * 0.35 + 8, 30), 90),
    '--sidebar-foreground': hslCss(h, 22, 14),
    '--sidebar-primary': hslCss(h, Math.min(sat + 28, 82), 34),
    '--sidebar-primary-foreground': hslCss(h, 12, 98),
    '--sidebar-accent': hslCss(h, Math.min(sat * 0.28 + 6, 24), 84),
    '--sidebar-accent-foreground': hslCss(h, 22, 14),
    '--sidebar-border': hslCss(h, Math.min(sat * 0.22 + 5, 20), 78),
    '--sidebar-ring': hslCss(h, Math.min(sat + 15, 55), 52),
    '--accent-orange': hslCss(h, Math.min(sat + 18, 72), 40),
    '--accent-orange-bright': hslCss(h, Math.min(sat + 26, 78), 48),
    '--chart-1': chartHue(h, 0),
    '--chart-2': chartHue(h, 18),
    '--chart-3': chartHue(h, -18),
    '--chart-4': chartHue(h, 36),
    '--chart-5': chartHue(h, -36),
  }
}

function buildDark(h: number, s: number): Record<string, string> {
  const sat = Math.max(s, 18)
  return {
    '--background': hslCss(h, 14, 10),
    '--foreground': hslCss(h, 28, 92),
    '--card': hslCss(h, 12, 14),
    '--card-foreground': hslCss(h, 25, 92),
    '--popover': hslCss(h, 12, 14),
    '--popover-foreground': hslCss(h, 25, 92),
    '--primary': hslCss(h, Math.min(sat + 12, 58), 62),
    '--primary-foreground': hslCss(h, 18, 10),
    '--secondary': hslCss(h, 10, 18),
    '--secondary-foreground': hslCss(h, 25, 92),
    '--muted': hslCss(h, 10, 18),
    '--muted-foreground': hslCss(h, 12, 68),
    '--accent': hslCss(h, 10, 20),
    '--accent-foreground': hslCss(h, 25, 92),
    '--border': hslCss(h, 12, 24),
    '--input': hslCss(h, 12, 24),
    '--ring': hslCss(h, Math.min(sat, 45), 48),
    '--sidebar': hslCss(h, 12, 11),
    '--sidebar-foreground': hslCss(h, 28, 92),
    '--sidebar-primary': hslCss(h, Math.min(sat + 12, 58), 62),
    '--sidebar-primary-foreground': hslCss(h, 18, 10),
    '--sidebar-accent': hslCss(h, 10, 18),
    '--sidebar-accent-foreground': hslCss(h, 25, 92),
    '--sidebar-border': hslCss(h, 12, 24),
    '--sidebar-ring': hslCss(h, Math.min(sat, 45), 48),
    '--accent-orange': hslCss(h, Math.min(sat + 8, 52), 68),
    '--accent-orange-bright': hslCss(h, Math.min(sat + 12, 58), 76),
    '--chart-1': chartHue(h, 0),
    '--chart-2': chartHue(h, 22),
    '--chart-3': chartHue(h, -22),
    '--chart-4': chartHue(h, 44),
    '--chart-5': chartHue(h, -44),
  }
}

export function buildAccentCssBlocks(hex: string): {
  light: string
  dark: string
} {
  const rgb = hexToRgb(hex)
  if (!rgb) return { light: '', dark: '' }
  let { h, s } = rgbToHsl(rgb.r, rgb.g, rgb.b)
  if (s < 6) {
    h = 43
    s = 22
  }
  const lightVars = buildLight(h, s)
  const darkVars = buildDark(h, s)
  const toDecl = (o: Record<string, string>) =>
    Object.entries(o)
      .map(([k, v]) => `${k}: ${v}`)
      .join(';')
  return {
    light: toDecl(lightVars),
    dark: toDecl(darkVars),
  }
}
