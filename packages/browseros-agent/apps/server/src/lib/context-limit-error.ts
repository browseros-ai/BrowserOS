export function isContextLimitError(value: unknown): boolean {
  const haystack = stringifyError(value).toLowerCase()
  return (
    /context[_ -]?length[_ -]?exceeded/.test(haystack) ||
    /maximum context length/.test(haystack) ||
    /max(?:imum)? context/.test(haystack) ||
    /context window/.test(haystack) ||
    /too many tokens/.test(haystack) ||
    /input (?:is )?too (?:large|long)/.test(haystack) ||
    /token limit/.test(haystack) ||
    /exceeds? [\w\s-]*tokens/.test(haystack)
  )
}

export function stringifyError(value: unknown): string {
  if (value instanceof Error) {
    const cause =
      'cause' in value && value.cause ? ` ${stringifyError(value.cause)}` : ''
    return `${value.name} ${value.message} ${value.stack ?? ''}${cause}`
  }
  if (typeof value === 'string') return value
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}
