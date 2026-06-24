/**
 * Mirror of the server-side `hexForSlug` colour mapping so the audit
 * screen's per-agent chips share colour with the homepage card border
 * and the BrowserOS tab group strip.
 */

const TAB_GROUP_COLORS = [
  'grey',
  'blue',
  'red',
  'yellow',
  'green',
  'pink',
  'purple',
  'cyan',
  'orange',
] as const

const TAB_GROUP_HEX: Record<(typeof TAB_GROUP_COLORS)[number], string> = {
  grey: '#6B7280',
  blue: '#2F6FE0',
  red: '#DC2626',
  yellow: '#F59E0B',
  green: '#10A37F',
  pink: '#DB2777',
  purple: '#7A5AF8',
  cyan: '#0EA5E9',
  orange: '#F26B2A',
}

function fnv1a(input: string): number {
  let hash = 0x811c9dc5
  for (let i = 0; i < input.length; i++) {
    hash ^= input.charCodeAt(i)
    hash =
      (hash +
        ((hash << 1) +
          (hash << 4) +
          (hash << 7) +
          (hash << 8) +
          (hash << 24))) >>>
      0
  }
  return hash
}

export function hexForSlug(slug: string): string {
  const idx = fnv1a(slug) % TAB_GROUP_COLORS.length
  return TAB_GROUP_HEX[TAB_GROUP_COLORS[idx] ?? 'grey']
}
