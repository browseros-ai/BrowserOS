export const extractPath = (args: Record<string, unknown>): string | null => {
  for (const key of ['path', 'file_path', 'filePath', 'file']) {
    if (typeof args[key] === 'string') return args[key] as string
  }
  return null
}

export const summarizeToolArgs = (args: Record<string, unknown>): string => {
  const path = extractPath(args)
  if (path) return `path=${path}`
  if (typeof args.command === 'string') return `command=${args.command}`
  if (typeof args.query === 'string') return `query=${args.query}`
  return Object.keys(args).join(', ')
}
