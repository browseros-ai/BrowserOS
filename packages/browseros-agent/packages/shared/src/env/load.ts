import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

import { type EnvMode, findEnvKeySpec } from './registry'

export type EnvValueSource = 'root-file' | 'process' | 'override'

export interface ResolvedEnv {
  rootDir: string
  mode: EnvMode
  values: Record<string, string>
  sources: Partial<Record<string, EnvValueSource>>
  targetFile: string
  demotedKeys: string[]
}

/** Parses the subset of dotenv syntax used by BrowserOS env files. */
export function parseEnvFile(text: string): Record<string, string> {
  const values: Record<string, string> = {}
  let index = 0

  while (index < text.length) {
    index = skipHorizontalWhitespace(text, index)
    if (index >= text.length) {
      break
    }
    if (isNewline(text[index])) {
      index = skipNewline(text, index)
      continue
    }
    if (text[index] === '#') {
      index = skipLine(text, index)
      continue
    }

    const parsed = readAssignment(text, index)
    if (!parsed) {
      index = skipLine(text, index)
      continue
    }

    values[parsed.key] = parsed.value
    index = parsed.nextIndex
  }

  return values
}

function readAssignment(
  text: string,
  startIndex: number,
): { key: string; value: string; nextIndex: number } | null {
  let index = startIndex

  if (
    text.startsWith('export', index) &&
    isHorizontalWhitespace(text[index + 6])
  ) {
    index = skipHorizontalWhitespace(text, index + 6)
  }

  const keyMatch = /^[A-Za-z_][A-Za-z0-9_]*/.exec(text.slice(index))
  if (!keyMatch) {
    return null
  }

  const key = keyMatch[0]
  index += key.length
  index = skipHorizontalWhitespace(text, index)

  if (text[index] !== '=') {
    return null
  }

  index = skipHorizontalWhitespace(text, index + 1)

  if (text[index] === "'") {
    const quoted = readSingleQuotedValue(text, index, key)
    return {
      key,
      value: quoted.value,
      nextIndex: skipLine(text, quoted.nextIndex),
    }
  }

  if (text[index] === '"') {
    const quoted = readDoubleQuotedValue(text, index, key)
    return {
      key,
      value: quoted.value,
      nextIndex: skipLine(text, quoted.nextIndex),
    }
  }

  const valueStart = index
  while (index < text.length && !isNewline(text[index])) {
    index += 1
  }

  let rawValue = text.slice(valueStart, index)
  if (rawValue.endsWith('\r')) {
    rawValue = rawValue.slice(0, -1)
  }

  return {
    key,
    value: stripUnquotedInlineComment(rawValue).trim(),
    nextIndex: skipNewline(text, index),
  }
}

function skipHorizontalWhitespace(text: string, index: number): number {
  while (index < text.length && isHorizontalWhitespace(text[index])) {
    index += 1
  }
  return index
}

function isHorizontalWhitespace(char: string | undefined): boolean {
  return char === ' ' || char === '\t'
}

function isNewline(char: string | undefined): boolean {
  return char === '\n' || char === '\r'
}

function skipLine(text: string, index: number): number {
  while (index < text.length && !isNewline(text[index])) {
    index += 1
  }
  return skipNewline(text, index)
}

function skipNewline(text: string, index: number): number {
  if (text[index] === '\r' && text[index + 1] === '\n') {
    return index + 2
  }
  if (isNewline(text[index])) {
    return index + 1
  }
  return index
}

function readSingleQuotedValue(
  text: string,
  quoteIndex: number,
  key: string,
): { value: string; nextIndex: number } {
  const end = text.indexOf("'", quoteIndex + 1)
  if (end === -1) {
    throw new Error(`Unterminated quoted value for ${key}`)
  }

  return { value: text.slice(quoteIndex + 1, end), nextIndex: end + 1 }
}

function readDoubleQuotedValue(
  text: string,
  quoteIndex: number,
  key: string,
): { value: string; nextIndex: number } {
  let result = ''

  for (let index = quoteIndex + 1; index < text.length; index += 1) {
    const char = text[index]
    if (char === '"') {
      return { value: result, nextIndex: index + 1 }
    }

    if (char !== '\\') {
      result += char
      continue
    }

    const decodedEscape = readEscapeSequence(text, index)
    result += decodedEscape.value
    index = decodedEscape.nextIndex
  }

  throw new Error(`Unterminated quoted value for ${key}`)
}

/** Loads the mode-specific root env file, returning an empty layer when absent. */
export function loadRootEnvFile(
  rootDir: string,
  mode: EnvMode,
): Record<string, string> {
  return loadOptionalEnvFile(join(rootDir, `.env.${mode}`))
}

/** Resolves root file, process, and override env layers with Bun auto-load demotion. */
export function resolveEnv(options: {
  rootDir: string
  mode: EnvMode
  overrides?: Record<string, string>
}): ResolvedEnv {
  const targetFile = join(options.rootDir, `.env.${options.mode}`)
  const fileValues = loadOptionalEnvFile(targetFile)
  const wrongAutoLoadValues = loadWrongAutoLoadValues(
    options.rootDir,
    options.mode,
  )
  const values: Record<string, string> = { ...fileValues }
  const sources: Partial<Record<string, EnvValueSource>> = {}
  const demotedKeys: string[] = []

  for (const key of Object.keys(fileValues)) {
    sources[key] = 'root-file'
  }

  for (const [key, value] of Object.entries(process.env)) {
    if (value === undefined) {
      continue
    }

    // Bun may prefill process.env from the wrong root env file before this loader runs.
    if (
      shouldDemoteAutoLoadedValue(key, value, fileValues, wrongAutoLoadValues)
    ) {
      demotedKeys.push(key)
      continue
    }

    values[key] = value
    sources[key] = 'process'
  }

  for (const [key, value] of Object.entries(options.overrides ?? {})) {
    values[key] = value
    sources[key] = 'override'
  }

  return {
    rootDir: options.rootDir,
    mode: options.mode,
    values,
    sources,
    targetFile,
    demotedKeys,
  }
}

/** Returns trimmed required values or throws one registry-sourced missing-key error. */
export function requireEnv(
  resolved: ResolvedEnv,
  keys: string[],
): Record<string, string> {
  const result: Record<string, string> = {}
  const missing: string[] = []

  for (const key of keys) {
    const value = resolved.values[key]
    if (value === undefined || value.trim() === '') {
      missing.push(key)
      continue
    }

    result[key] = value.trim()
  }

  if (missing.length > 0) {
    throw new Error(formatMissingEnvError(resolved.mode, missing))
  }

  validateRequiredValues(keys, result)

  return result
}

function readEscapeSequence(
  value: string,
  slashIndex: number,
): { value: string; nextIndex: number } {
  const escaped = value[slashIndex + 1]
  if (escaped === undefined) {
    return { value: '\\', nextIndex: slashIndex }
  }

  switch (escaped) {
    case 'a':
      return { value: '\u0007', nextIndex: slashIndex + 1 }
    case 'b':
      return { value: '\b', nextIndex: slashIndex + 1 }
    case 'f':
      return { value: '\f', nextIndex: slashIndex + 1 }
    case 'n':
      return { value: '\n', nextIndex: slashIndex + 1 }
    case 'r':
      return { value: '\r', nextIndex: slashIndex + 1 }
    case 't':
      return { value: '\t', nextIndex: slashIndex + 1 }
    case 'v':
      return { value: '\v', nextIndex: slashIndex + 1 }
    case '"':
    case '\\':
      return { value: escaped, nextIndex: slashIndex + 1 }
    case 'x':
      return readHexEscape(value, slashIndex, 2)
    case 'u':
      return readHexEscape(value, slashIndex, 4)
    case 'U':
      return readHexEscape(value, slashIndex, 8)
    default:
      return { value: escaped, nextIndex: slashIndex + 1 }
  }
}

function readHexEscape(
  value: string,
  slashIndex: number,
  length: number,
): { value: string; nextIndex: number } {
  const start = slashIndex + 2
  const hex = value.slice(start, start + length)
  if (hex.length !== length || !/^[0-9a-fA-F]+$/.test(hex)) {
    return { value: value[slashIndex + 1] ?? '', nextIndex: slashIndex + 1 }
  }

  const codePoint = Number.parseInt(hex, 16)
  if (codePoint > 0x10ffff) {
    return {
      value: value.slice(slashIndex, start + length),
      nextIndex: start + length - 1,
    }
  }

  return {
    value: String.fromCodePoint(codePoint),
    nextIndex: start + length - 1,
  }
}

function stripUnquotedInlineComment(value: string): string {
  for (let index = 0; index < value.length; index += 1) {
    if (value[index] === '#' && (index === 0 || /\s/.test(value[index - 1]))) {
      return value.slice(0, index)
    }
  }

  return value
}

function loadOptionalEnvFile(path: string): Record<string, string> {
  if (!existsSync(path)) {
    return {}
  }

  return parseEnvFile(readFileSync(path, 'utf8'))
}

function loadWrongAutoLoadValues(
  rootDir: string,
  mode: EnvMode,
): Array<Record<string, string>> {
  const otherMode = mode === 'development' ? 'production' : 'development'
  return ['.env', '.env.local', `.env.${otherMode}`].map((file) =>
    loadOptionalEnvFile(join(rootDir, file)),
  )
}

function shouldDemoteAutoLoadedValue(
  key: string,
  processValue: string,
  targetValues: Record<string, string>,
  wrongAutoLoadValues: Array<Record<string, string>>,
): boolean {
  const targetValue = targetValues[key]
  if (targetValue === processValue) {
    return false
  }

  return wrongAutoLoadValues.some((values) => values[key] === processValue)
}

function validateRequiredValues(
  keys: string[],
  values: Record<string, string>,
): void {
  for (const key of keys) {
    const spec = findEnvKeySpec(key)
    if (!spec) {
      continue
    }

    const result = spec.schema.safeParse(values[key])
    if (!result.success) {
      const section = ` (section: ${spec.section})`
      const reason = result.error.issues
        .map((issue) => issue.message)
        .join('; ')
      throw new Error(`Invalid env: ${key}${section}. ${reason}`)
    }
  }
}

function formatMissingEnvError(mode: EnvMode, keys: string[]): string {
  const details = keys.map((key) => formatMissingEnvDetail(mode, key))

  if (details.length === 1) {
    return `Missing required env: ${details[0]}`
  }

  return `Missing required env:\n${details.map((detail) => `- ${detail}`).join('\n')}`
}

function formatMissingEnvDetail(mode: EnvMode, key: string): string {
  const spec = findEnvKeySpec(key)
  const section = spec ? ` (section: ${spec.section})` : ''
  return `${key}${section}. Set it in .env.${mode} at the monorepo root or export it in the environment.`
}
