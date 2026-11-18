/** How the designed screen writes times, clock times and token counts. Tokens always carry "≈". */

const numbers = new Intl.NumberFormat('en-US')

export function count(value: number): string {
  return numbers.format(value)
}

/** `412 ms`, `1.85 s`, `70.9 s`, `214 s`, `12 min 4 s`: more digits while they still mean something. */
export function seconds(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`
  if (ms < 10_000) return `${(ms / 1000).toFixed(2)} s`
  if (ms < 100_000) return `${(ms / 1000).toFixed(1)} s`
  if (ms < 600_000) return `${Math.round(ms / 1000)} s`
  const minutes = Math.floor(ms / 60_000)
  return `${minutes} min ${Math.round((ms % 60_000) / 1000)} s`
}

/** `10:14:02`, or `10:14:02.1` with tenths; 24-hour local time, as the design writes it. */
export function clock(at: string | number, tenths = false): string {
  const date = new Date(at)
  const pad = (n: number) => String(n).padStart(2, '0')
  const base = `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`
  return tenths ? `${base}.${Math.floor(date.getMilliseconds() / 100)}` : base
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${count(n)} ${n === 1 ? one : many}`
}
