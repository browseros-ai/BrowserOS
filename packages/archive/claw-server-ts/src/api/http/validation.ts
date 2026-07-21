export function positiveInteger(raw: string): number | null {
  if (!/^\d+$/.test(raw)) return null
  const value = Number(raw)
  return Number.isSafeInteger(value) && value > 0 ? value : null
}

/** `undefined` means absent; `false` means present but outside the accepted integer range. */
export function optionalInteger(
  raw: string | undefined,
  minimum: number,
  maximum = Number.MAX_SAFE_INTEGER,
): number | undefined | false {
  if (raw === undefined) return undefined
  if (!/^\d+$/.test(raw)) return false
  const value = Number(raw)
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    return false
  }
  return value
}
