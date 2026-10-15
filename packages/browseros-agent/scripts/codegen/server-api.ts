/**
 * Codegen for the canonical BrowserOS server API. Reads the OpenAPI spec at
 * `contracts/server-api/openapi.yaml` and emits language-specific DTOs:
 * TypeScript models into `packages/server-api/src/generated` and Rust
 * models into `crates/server-api/src/generated`, plus type-only OpenAPI
 * operations into `packages/server-api-client/src/generated`. DTO models are
 * grouped by their contract schema files and flat-re-exported for consumer compatibility.
 * Neither tree is ever hand-edited — change the spec, then regenerate with
 * `bun run codegen:server-api`.
 *
 * The generator runs in Docker with a pinned image and the output is
 * normalized (trailing whitespace stripped, generated Rust run through
 * rustfmt) so the emitted trees are byte-identical across machines.
 * That determinism is what lets `--check` (`bun run
 * codegen:server-api:check`, the CI drift gate) compare a fresh
 * generation against the committed trees byte for byte.
 */

import { spawnSync } from 'node:child_process'
import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, relative, resolve } from 'node:path'
import { parse } from 'yaml'

const root = join(import.meta.dir, '../..')
const image = 'openapitools/openapi-generator-cli:v7.22.0'
const check = process.argv.includes('--check')

interface GeneratedTrees {
  typescript: string
  rust: string
  client: string
}

interface OpenApiSchemasDocument {
  components?: {
    schemas?: Record<string, { $ref?: unknown }>
  }
}

/**
 * Derives model ownership from the contract's schema file layout and checks
 * that the generator emitted exactly the contract's model set.
 */
export function extractModelGroups(
  openApiSource: string,
  generatedModels: Iterable<string>,
): Record<string, string> {
  const document = parse(openApiSource) as OpenApiSchemasDocument
  const schemas = document.components?.schemas ?? {}
  const groups: Record<string, string> = {}

  for (const model of Object.keys(schemas).toSorted()) {
    const ref = schemas[model]?.$ref
    const match =
      typeof ref === 'string'
        ? ref.match(
            /^\.\/schemas\/([a-z][a-z0-9_]*)\.yaml#\/([A-Za-z][A-Za-z0-9]*)$/,
          )
        : null
    if (!match || match[2] !== model) {
      throw new Error(
        `${model} schema ref must match ./schemas/<group>.yaml#/<model>`,
      )
    }
    groups[model] = match[1] as string
  }

  const generated = new Set(generatedModels)
  for (const model of [...generated].toSorted()) {
    if (!(model in groups)) {
      throw new Error(`Generated model ${model} has no schema group`)
    }
  }
  for (const model of Object.keys(groups)) {
    if (!generated.has(model)) {
      throw new Error(`OpenAPI schema ${model} has no generated model`)
    }
  }

  return groups
}

interface ParsedRustModel {
  header: string
  imports: string[]
  body: string
}

export function mergeRustModels(sources: Record<string, string>): string {
  const models = Object.keys(sources).toSorted()
  if (models.length === 0) throw new Error('Cannot merge an empty Rust group')

  const parsed = models.map((model) => ({
    model,
    source: parseRustModel(model, sources[model] as string),
  }))
  const header = parsed[0]?.source.header as string
  for (const { model, source } of parsed.slice(1)) {
    if (source.header !== header) {
      throw new Error(`Rust model ${model} has a different generated header`)
    }
  }

  const imports = new Set(parsed.flatMap(({ source }) => source.imports))
  // Array-of-free-JSON oneOf variants otherwise contain `::` in their Rust
  // identifier. Untagged variant names have no effect on the wire format.
  return `${header}\n\n${[...imports].toSorted().join('\n')}\n\n${parsed
    .map(({ model, source }) =>
      namespaceRustModelHelpers(
        model,
        source.body.replaceAll(
          /ArrayVec(?:serde_json|models)::([A-Za-z][A-Za-z0-9]*)\(/g,
          'Array$1(',
        ),
      ),
    )
    .join('\n\n')}\n`
}

export function emitRustModuleIndex(groups: Iterable<string>): string {
  return `${[...new Set(groups)]
    .toSorted()
    .map((group) => `pub mod ${group};\npub use self::${group}::*;`)
    .join('\n')}\n`
}

interface ParsedTypeScriptImport {
  typeOnly: boolean
  specifiers: string[]
  target: string
}

interface ParsedTypeScriptModel {
  preamble: string
  imports: ParsedTypeScriptImport[]
  body: string
}

export function mergeTypeScriptModels(
  sources: Record<string, string>,
  modelGroups: Readonly<Record<string, string>>,
): string {
  const models = Object.keys(sources).toSorted()
  if (models.length === 0) {
    throw new Error('Cannot merge an empty TypeScript group')
  }

  const sourceGroups = new Set(models.map((model) => modelGroups[model]))
  if (sourceGroups.has(undefined) || sourceGroups.size !== 1) {
    throw new Error('TypeScript model sources must belong to one known group')
  }
  const sourceGroup = [...sourceGroups][0] as string
  const parsed = models.map((model) => ({
    model,
    source: parseTypeScriptModel(model, sources[model] as string),
  }))
  const preamble = parsed[0]?.source.preamble as string
  for (const { model, source } of parsed.slice(1)) {
    if (source.preamble !== preamble) {
      throw new Error(
        `TypeScript model ${model} has a different generated preamble`,
      )
    }
  }

  const imports = collectTypeScriptImports(parsed, modelGroups, sourceGroup)

  const importBlock = [...imports.entries()]
    .toSorted(([left], [right]) => compareTypeScriptImportKeys(left, right))
    .map(([key, specifiers]) => {
      const [target, kind] = key.split('\0') as [string, 'type' | 'value']
      return formatTypeScriptImport(
        target,
        kind === 'type',
        [...specifiers].toSorted(),
      )
    })
    .join('\n')

  const importSection = importBlock ? `${importBlock}\n\n` : ''
  return `${preamble}\n\n${importSection}${parsed
    .map(({ source }) => source.body)
    .join('\n\n')}\n`
}

function collectTypeScriptImports(
  models: Array<{ model: string; source: ParsedTypeScriptModel }>,
  modelGroups: Readonly<Record<string, string>>,
  sourceGroup: string,
): Map<string, Set<string>> {
  const imports = new Map<string, Set<string>>()
  for (const { model, source } of models) {
    for (const declaration of source.imports) {
      const target = rewriteTypeScriptImportTarget(
        model,
        declaration.target,
        modelGroups,
        sourceGroup,
      )
      if (!target) continue

      const key = `${target}\0${declaration.typeOnly ? 'type' : 'value'}`
      const specifiers = imports.get(key) ?? new Set<string>()
      for (const specifier of declaration.specifiers) specifiers.add(specifier)
      imports.set(key, specifiers)
    }
  }
  return imports
}

function rewriteTypeScriptImportTarget(
  model: string,
  target: string,
  modelGroups: Readonly<Record<string, string>>,
  sourceGroup: string,
): string | null {
  if (target === '../runtime.js') return target
  const importedModel = target.match(
    /^\.\/([A-Za-z][A-Za-z0-9]*)(?:\.js)?$/,
  )?.[1]
  const importedGroup = importedModel ? modelGroups[importedModel] : undefined
  if (!importedModel || !importedGroup) {
    throw new Error(
      `TypeScript model ${model} imports unknown model target ${target}`,
    )
  }
  return importedGroup === sourceGroup ? null : `./${importedGroup}`
}

export function emitTypeScriptModelIndex(groups: Iterable<string>): string {
  return `${[...new Set(groups)]
    .toSorted()
    .map((group) => `export * from './models/${group}'`)
    .join('\n')}\n`
}

function parseRustModel(model: string, source: string): ParsedRustModel {
  const normalized = source.trimEnd()
  const header = normalized.match(/^\/\*[\s\S]*?\*\//)?.[0]
  if (!header) throw new Error(`Rust model ${model} has no generated header`)

  const remainder = normalized.slice(header.length).trimStart()
  const lines = remainder.split('\n')
  const imports: string[] = []
  let bodyStart = 0
  for (; bodyStart < lines.length; bodyStart += 1) {
    const line = lines[bodyStart] as string
    if (line.length === 0) continue
    if (line.startsWith('use ') && line.endsWith(';')) {
      imports.push(line)
      continue
    }
    break
  }
  if (imports.length === 0) {
    throw new Error(`Rust model ${model} has no generated imports`)
  }

  return {
    header,
    imports,
    body: lines.slice(bodyStart).join('\n').trim(),
  }
}

/**
 * Per-model files namespace inline-schema helpers implicitly. A merged group
 * prefixes those helpers and keeps their former module path as an alias.
 */
function namespaceRustModelHelpers(model: string, body: string): string {
  const helpers = [
    ...body.matchAll(/^pub (?:struct|enum|type) ([A-Za-z][A-Za-z0-9]*)\b/gm),
  ]
    .map((match) => match[1] as string)
    .filter((name) => name !== model)
  if (helpers.length === 0) return body

  let namespaced = body
  for (const helper of helpers) {
    namespaced = namespaced.replaceAll(
      new RegExp(`\\b${helper}\\b`, 'g'),
      `${model}${helper}`,
    )
  }
  const legacyModule = model
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .toLowerCase()
  const aliases = helpers
    .map((helper) => `    pub use super::${model}${helper} as ${helper};`)
    .join('\n')
  return `${namespaced}\n\npub mod ${legacyModule} {\n${aliases}\n}`
}

function parseTypeScriptModel(
  model: string,
  source: string,
): ParsedTypeScriptModel {
  const normalized = source.trimEnd()
  const preamble = normalized.match(
    /^\/\* tslint:disable \*\/\n\/\* eslint-disable \*\/\n\/\*\*[\s\S]*?\*\//,
  )?.[0]
  if (!preamble) {
    throw new Error(`TypeScript model ${model} has no generated preamble`)
  }

  const imports: ParsedTypeScriptImport[] = []
  let cursor = preamble.length
  while (normalized[cursor] === '\n') cursor += 1
  while (normalized.startsWith('import ', cursor)) {
    const end = normalized.indexOf(';\n', cursor)
    if (end === -1) {
      throw new Error(`TypeScript model ${model} has an unrecognized import`)
    }
    const statement = normalized.slice(cursor, end + 1)
    const match = statement.match(
      /^import( type)? \{([\s\S]*?)\} from '([^']+)';$/,
    )
    if (!match) {
      throw new Error(`TypeScript model ${model} has an unrecognized import`)
    }
    const specifiers = (match[2] as string)
      .split(',')
      .map((specifier) => specifier.trim())
      .filter(Boolean)
    if (specifiers.length === 0) {
      throw new Error(`TypeScript model ${model} has an empty import`)
    }
    imports.push({
      typeOnly: Boolean(match[1]),
      specifiers,
      target: match[3] as string,
    })
    cursor = end + 2
  }
  while (normalized[cursor] === '\n') cursor += 1
  const body = normalized.slice(cursor).trim()
  if (!body) throw new Error(`TypeScript model ${model} has no model body`)
  if (/^import\b/m.test(body)) {
    throw new Error(`TypeScript model ${model} has an unrecognized import`)
  }

  return {
    preamble,
    imports,
    body,
  }
}

function compareTypeScriptImportKeys(left: string, right: string): number {
  const [leftTarget, leftKind] = left.split('\0')
  const [rightTarget, rightKind] = right.split('\0')
  if (leftTarget === '../runtime.js' && rightTarget !== '../runtime.js')
    return -1
  if (rightTarget === '../runtime.js' && leftTarget !== '../runtime.js')
    return 1
  const targetOrder = (leftTarget as string).localeCompare(
    rightTarget as string,
  )
  if (targetOrder !== 0) return targetOrder
  return leftKind === rightKind ? 0 : leftKind === 'type' ? -1 : 1
}

function formatTypeScriptImport(
  target: string,
  typeOnly: boolean,
  specifiers: string[],
): string {
  const keyword = typeOnly ? 'import type' : 'import'
  if (specifiers.length === 1) {
    return `${keyword} { ${specifiers[0]} } from '${target}';`
  }
  return `${keyword} {\n${specifiers.map((specifier) => `    ${specifier},`).join('\n')}\n} from '${target}';`
}

/**
 * Collapse file refs before invoking OpenAPI Generator. Its external-ref resolver
 * duplicates models used by cross-file unions; canonical component refs preserve
 * one identity for each DTO while the authored contract stays split and readable.
 */
function bundleSpec(): unknown {
  const specPath = join(root, 'contracts/server-api/openapi.yaml')
  const documents = new Map<string, unknown>()
  function read(path: string): unknown {
    if (!documents.has(path))
      documents.set(path, parse(readFileSync(path, 'utf8')))
    return documents.get(path)
  }
  const document = read(specPath) as {
    components: { schemas: Record<string, { $ref: string }> }
  }
  const identities = new Map<string, string>()
  function target(ref: string, from: string): [string, string] {
    const [file, fragment = ''] = ref.split('#')
    return [file ? resolve(dirname(from), file) : from, fragment]
  }
  for (const [name, schema] of Object.entries(document.components.schemas)) {
    identities.set(target(schema.$ref, specPath).join('#'), name)
  }
  function expand(value: unknown, from: string, definition = false): unknown {
    if (Array.isArray(value)) return value.map((item) => expand(item, from))
    if (!value || typeof value !== 'object') return value
    const record = value as Record<string, unknown>
    if (typeof record.$ref === 'string') {
      const [file, pointer] = target(record.$ref, from)
      const identity = identities.get(`${file}#${pointer}`)
      if (identity && !definition)
        return { $ref: `#/components/schemas/${identity}` }
      let resolved = read(file)
      for (const key of pointer.split('/').slice(1)) {
        resolved = (resolved as Record<string, unknown>)[
          key.replaceAll('~1', '/').replaceAll('~0', '~')
        ]
      }
      return expand(resolved, file)
    }
    return Object.fromEntries(
      Object.entries(record).map(([key, item]) => [key, expand(item, from)]),
    )
  }
  const bundled = expand(document, specPath) as {
    components: { schemas: Record<string, unknown> }
  }
  bundled.components.schemas = Object.fromEntries(
    Object.entries(document.components.schemas).map(([name, schema]) => [
      name,
      expand(schema, specPath, true),
    ]),
  )
  // The discriminator is redundant with each literal tag and external mapping
  // URIs confuse the Rust generator after bundling. The oneOf stays intact.
  for (const schema of Object.values(bundled.components.schemas)) {
    if (schema && typeof schema === 'object')
      delete (schema as Record<string, unknown>).discriminator
  }
  return bundled
}

function runGenerator(outputRoot: string): GeneratedTrees {
  writeFileSync(join(outputRoot, 'spec.json'), JSON.stringify(bundleSpec()))
  const templates = join(outputRoot, 'templates')
  cpSync(
    join(root, 'scripts/codegen/templates/claw-api/typescript-fetch'),
    templates,
    { recursive: true },
  )
  // The upstream union template emits runtime conversion functions even in
  // models-only mode. Keep this contract's unions type-only, like its DTOs.
  writeFileSync(
    join(templates, 'modelOneOf.mustache'),
    `{{#hasImports}}
{{#oneOfArrays}}
import type { {{{.}}} } from './{{.}}{{importFileExtension}}';
{{/oneOfArrays}}
{{#oneOfModels}}
import type { {{{.}}} } from './{{.}}{{importFileExtension}}';
{{/oneOfModels}}
{{/hasImports}}
{{>modelOneOfInterfaces}}
`,
  )

  // Run the container as the invoking user so the generated files on
  // the bind mount aren't root-owned on Linux hosts.
  const mount = `${process.getuid?.() ?? 1000}:${process.getgid?.() ?? 1000}`
  const common = [
    'run',
    '--rm',
    '--user',
    mount,
    '-v',
    `${root}:/local:ro`,
    '-v',
    `${outputRoot}:/out`,
    image,
    'generate',
    '-i',
    '/out/spec.json',
    '--generate-alias-as-model',
  ]

  runDocker([
    ...common,
    '-g',
    'typescript-fetch',
    '-t',
    '/out/templates',
    '-o',
    '/out/typescript',
    '--global-property',
    'models,modelDocs=false,modelTests=false',
    // The upstream reserved-word list escapes this SDK field to _transient.
    // These models are the wire payloads, so preserve its real property name.
    '--reserved-words-mappings',
    'transient=transient',
    '--additional-properties',
    'supportsES6=true,typescriptThreePlus=true,importFileExtension=,disallowAdditionalPropertiesIfNotPresent=false',
  ])
  runDocker([
    ...common,
    '-g',
    'rust',
    '-o',
    '/out/rust',
    '--global-property',
    'models,supportingFiles,modelDocs=false,modelTests=false,apis=false,apiDocs=false,apiTests=false',
    '--additional-properties',
    'packageName=server-api,packageVersion=1.0.0',
  ])

  const typescript = join(outputRoot, 'typescript')
  const rust = join(outputRoot, 'rust/src/models')
  const client = join(outputRoot, 'client')
  rmSync(join(typescript, '.openapi-generator'), {
    recursive: true,
    force: true,
  })
  rmSync(join(typescript, '.openapi-generator-ignore'), { force: true })
  const groups = groupGeneratedModels(typescript, rust)
  for (const file of listFiles(typescript).filter((path) =>
    path.endsWith('.ts'),
  )) {
    const path = join(typescript, file)
    const source = retainTypeScriptPatterns(readFileSync(path, 'utf8'))
    const normalized = `${source
      .split('\n')
      .map((line) =>
        line.trimStart().startsWith('*')
          ? line.trimEnd()
          : line
              .trimEnd()
              .replace(/\bany\b/g, 'unknown')
              .replace(/unknown \| unknown/g, 'unknown'),
      )
      .join('\n')
      .trimEnd()}\n`
    writeFileSync(path, normalized)
    // Match the repository's pre-commit formatter so committing generated DTOs
    // cannot silently make the byte-for-byte drift gate stale.
    runFormatter(path, join(root, 'packages/server-api/src/generated', file))
  }
  writeFileSync(join(typescript, 'index.ts'), emitTypeScriptModelIndex(groups))
  mkdirSync(client, { recursive: true })
  const clientOutput = join(client, 'openapi.ts')
  runOpenApiTypescript(clientOutput)
  writeFileSync(
    clientOutput,
    `// This file is generated from contracts/server-api/openapi.yaml. Do not edit.\n${retainTypeScriptPatterns(readFileSync(clientOutput, 'utf8'))}`,
  )
  runFormatter(
    clientOutput,
    join(root, 'packages/server-api-client/src/generated/openapi.ts'),
  )
  const chunkValidator = join(client, 'ui-message-chunk.ts')
  writeFileSync(chunkValidator, emitChunkValidator())
  runFormatter(
    chunkValidator,
    join(root, 'packages/server-api-client/src/generated/ui-message-chunk.ts'),
  )
  for (const file of listFiles(rust).filter((path) => path.endsWith('.rs'))) {
    const result = spawnSync(
      'rustfmt',
      ['--edition', '2024', join(rust, file)],
      {
        stdio: 'inherit',
      },
    )
    if (result.error) throw result.error
    if (result.status !== 0) {
      throw new Error(`rustfmt exited with status ${result.status}`)
    }
  }

  return {
    typescript,
    rust,
    client,
  }
}

/**
 * OpenAPI represents data-${string} with a regex; both upstream TS generators
 * otherwise widen it to string, preventing callers from narrowing fixed tags.
 * Keep the schema's explicit TS refinement in both model and operation output.
 */
function retainTypeScriptPatterns(source: string): string {
  const spec = bundleSpec() as {
    components: { schemas: Record<string, Record<string, unknown>> }
  }
  for (const [model, schema] of Object.entries(spec.components.schemas)) {
    const properties = (schema.properties ?? {}) as Record<
      string,
      Record<string, unknown>
    >
    for (const [property, shape] of Object.entries(properties)) {
      const refinement = shape['x-typescript-type']
      if (typeof refinement !== 'string') continue
      source = source.replace(
        new RegExp(
          `((?:export interface ${model}|${model}:) \\{[\\s\\S]*?\\n\\s*${property}\\??:) string\\b`,
        ),
        (_match, prefix: string) => `${prefix} ${refinement}`,
      )
    }
  }
  return source
}

/** Generate payload validation and known-tag detection from the same union. */
function emitChunkValidator(): string {
  const spec = bundleSpec() as {
    components: { schemas: Record<string, Record<string, unknown>> }
  }
  const schemas = spec.components.schemas
  let variable = 0
  function check(shape: Record<string, unknown>, value: string): string {
    if (typeof shape.$ref === 'string')
      return check(
        schemas[shape.$ref.split('/').at(-1) as string] as Record<
          string,
          unknown
        >,
        value,
      )
    if (Array.isArray(shape.enum)) {
      const candidate = `allowed${variable++}`
      return `${JSON.stringify(shape.enum)}.some((${candidate}) => ${candidate} === ${value})`
    }
    if (typeof shape.pattern === 'string')
      return `typeof ${value} === 'string' && ${new RegExp(shape.pattern).toString()}.test(${value})`
    if (shape.type === 'string' || shape.type === 'boolean')
      return `typeof ${value} === '${shape.type}'`
    if (shape.type === 'number' || shape.type === 'integer')
      return `(typeof ${value} === 'number' && Number.isFinite(${value}))`
    if (shape.type === 'object') {
      const terms = [`isRecord(${value})`]
      const required = new Set(shape.required as string[] | undefined)
      for (const [key, prop] of Object.entries(
        (shape.properties ?? {}) as Record<string, Record<string, unknown>>,
      )) {
        const field = /^[A-Za-z_$][\w$]*$/.test(key)
          ? `${value}.${key}`
          : `${value}[${JSON.stringify(key)}]`
        const condition = check(prop, field)
        terms.push(
          required.has(key)
            ? `${JSON.stringify(key)} in ${value} && ${condition}`
            : `(!(${JSON.stringify(key)} in ${value}) || ${condition})`,
        )
      }
      if (
        shape.additionalProperties &&
        typeof shape.additionalProperties === 'object' &&
        Object.keys(shape.additionalProperties).length > 0
      ) {
        const item = `item${variable++}`
        terms.push(
          `Object.values(${value}).every((${item}) => ${check(shape.additionalProperties as Record<string, unknown>, item)})`,
        )
      }
      return `(${terms.join(' && ')})`
    }
    return 'true'
  }
  const variants = schemas.UiMessageChunk?.oneOf as Array<{ $ref: string }>
  const tags: string[] = []
  const patternedCases: string[] = []
  const patternedTypes: string[] = []
  const cases = variants
    .map((variant) => {
      const shape = schemas[variant.$ref.split('/').at(-1) as string] as Record<
        string,
        unknown
      >
      const typeSchema = (
        shape.properties as Record<string, Record<string, unknown>>
      ).type
      const tag = (typeSchema?.enum as string[] | undefined)?.[0]
      if (!tag) {
        if (typeof typeSchema?.pattern !== 'string')
          throw new Error('A UI chunk must declare a literal or patterned type')
        patternedCases.push(check(shape, 'value'))
        patternedTypes.push(check(typeSchema, 'type'))
        return ''
      }
      tags.push(tag)
      return `case ${JSON.stringify(tag)}: return ${check(shape, 'value')}`
    })
    .join('\n')
  return `// Generated from contracts/server-api/openapi.yaml. Do not edit.
import type { UiMessageChunk } from '@browseros/server-api'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
/** Separates future tags from malformed known payloads, including every data-* name. */
export function isKnownUiMessageChunkType(type: string): boolean {
  return ${[`${JSON.stringify(tags)}.includes(type)`, ...patternedTypes].join(' || ')}
}
/** Validates stream payloads without dropping provider metadata or extension fields. */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: generated cases validate the complete chunk union.
export function isUiMessageChunk(value: unknown): value is UiMessageChunk {
  if (!isRecord(value)) return false
  switch(value.type) {
    ${cases}
    default: return ${patternedCases.join(' || ') || 'false'}
  }
}
`
}

function runOpenApiTypescript(output: string): void {
  // openapi-typescript 7 uses the TypeScript 5 compiler API, while the
  // workspace's TypeScript 7 package intentionally no longer exports it.
  const result = spawnSync(
    'bunx',
    [
      '--package',
      'typescript@5.9.3',
      '--package',
      'openapi-typescript@7.13.0',
      'openapi-typescript',
      join(root, 'contracts/server-api/openapi.yaml'),
      '--output',
      output,
      '--default-non-nullable',
      'false',
    ],
    { stdio: 'inherit' },
  )
  if (result.error) throw result.error
  if (result.status !== 0) {
    throw new Error(`openapi-typescript exited with status ${result.status}`)
  }
}

function runFormatter(output: string, sourcePath: string): void {
  const result = spawnSync(
    'biome',
    ['format', `--stdin-file-path=${sourcePath}`],
    {
      encoding: 'utf8',
      input: readFileSync(output, 'utf8'),
      stdio: ['pipe', 'pipe', 'inherit'],
    },
  )
  if (result.error) throw result.error
  if (result.status !== 0) {
    throw new Error(`Biome exited with status ${result.status}`)
  }
  writeFileSync(output, result.stdout)
}

function groupGeneratedModels(typescript: string, rust: string): string[] {
  const typescriptModels = join(typescript, 'models')
  const typescriptFiles = listFiles(typescriptModels).filter(
    (file) => file.endsWith('.ts') && file !== 'index.ts',
  )
  const typescriptModelNames = typescriptFiles.map((file) => file.slice(0, -3))
  const openApiSource = readFileSync(
    join(root, 'contracts/server-api/openapi.yaml'),
    'utf8',
  )
  const modelGroups = extractModelGroups(openApiSource, typescriptModelNames)
  const groups = [...new Set(Object.values(modelGroups))].toSorted()
  const typescriptSources = sourcesByGroup(
    typescriptModelNames,
    modelGroups,
    (model) => readFileSync(join(typescriptModels, `${model}.ts`), 'utf8'),
  )

  const rustIndex = readFileSync(join(rust, 'mod.rs'), 'utf8')
  const rustFilesByModel = rustModelFilesByModel(rustIndex)
  extractModelGroups(openApiSource, Object.keys(rustFilesByModel))
  const rustSources = sourcesByGroup(
    Object.keys(rustFilesByModel),
    modelGroups,
    (model) =>
      normalizeRustModel(
        model,
        readFileSync(join(rust, rustFilesByModel[model] as string), 'utf8'),
      ),
  )

  for (const file of listFiles(typescriptModels).filter((file) =>
    file.endsWith('.ts'),
  )) {
    rmSync(join(typescriptModels, file))
  }
  for (const group of groups) {
    writeFileSync(
      join(typescriptModels, `${group}.ts`),
      mergeTypeScriptModels(
        typescriptSources[group] as Record<string, string>,
        modelGroups,
      ),
    )
  }
  for (const file of listFiles(rust).filter((file) => file.endsWith('.rs'))) {
    rmSync(join(rust, file))
  }
  for (const group of groups) {
    writeFileSync(
      join(rust, `${group}.rs`),
      mergeRustModels(rustSources[group] as Record<string, string>),
    )
  }
  writeFileSync(join(rust, 'mod.rs'), emitRustModuleIndex(groups))
  return groups
}

/** Preserve opaque messages, header maps and extension fields across Rust round trips. */
function normalizeRustModel(model: string, source: string): string {
  const spec = bundleSpec() as {
    components: { schemas: Record<string, Record<string, unknown>> }
  }
  const schema = spec.components.schemas[model]
  if (schema?.type !== 'object') return source
  const properties = schema.properties as Record<string, unknown> | undefined
  if (!properties || Object.keys(properties).length === 0) {
    const additional = schema.additionalProperties
    if (additional === true || (additional && typeof additional === 'object')) {
      const valueType =
        additional !== true &&
        (additional as Record<string, unknown>).type === 'string'
          ? 'String'
          : 'serde_json::Value'
      const header = source.match(/^\/\*[\s\S]*?\*\//)?.[0]
      return `${header}\n\nuse serde::{Deserialize, Serialize};\n\npub type ${model} = std::collections::HashMap<String, ${valueType}>;\n`
    }
  }
  if (schema.additionalProperties === true) {
    source = source.replace(
      `pub struct ${model} {`,
      `pub struct ${model} {\n    #[serde(flatten)]\n    pub additional_properties: std::collections::HashMap<String, serde_json::Value>,`,
    )
    source = source.replace(
      new RegExp(`(\\n {8}${model} \\{)`),
      '$1\n            additional_properties: Default::default(),',
    )
  } else if (schema.additionalProperties === false) {
    source = source.replace(
      `pub struct ${model} {`,
      `#[serde(deny_unknown_fields)]\npub struct ${model} {`,
    )
  }
  return source
}

function sourcesByGroup(
  models: string[],
  modelGroups: Readonly<Record<string, string>>,
  sourceForModel: (model: string) => string,
): Record<string, Record<string, string>> {
  const sources: Record<string, Record<string, string>> = {}
  for (const model of models.toSorted()) {
    const group = modelGroups[model]
    if (!group) throw new Error(`Generated model ${model} has no schema group`)
    const groupSources = sources[group] ?? {}
    groupSources[model] = sourceForModel(model)
    sources[group] = groupSources
  }
  return sources
}

function rustModelFilesByModel(indexSource: string): Record<string, string> {
  const entries = [
    ...indexSource.matchAll(
      /^pub use self::([a-z][a-z0-9_]*)::([A-Za-z][A-Za-z0-9]*);$/gm,
    ),
  ]
  if (entries.length === 0) {
    throw new Error('OpenAPI Generator Rust model index shape changed')
  }
  return Object.fromEntries(
    entries.map((match) => [match[2] as string, `${match[1] as string}.rs`]),
  )
}

function runDocker(args: string[]): void {
  const result = spawnSync('docker', args, { stdio: 'inherit' })
  if (result.error) throw result.error
  if (result.status !== 0) {
    throw new Error(`OpenAPI Generator exited with status ${result.status}`)
  }
}

function installGenerated(trees: GeneratedTrees): void {
  const typescriptTarget = join(root, 'packages/server-api/src/generated')
  const rustTarget = join(root, 'crates/server-api/src/generated')
  const clientTarget = join(root, 'packages/server-api-client/src/generated')

  rmSync(typescriptTarget, { recursive: true, force: true })
  cpSync(trees.typescript, typescriptTarget, {
    recursive: true,
    filter: (source) => {
      const path = relative(trees.typescript, source)
      return !path.startsWith('.openapi-generator')
    },
  })
  rmSync(join(typescriptTarget, '.openapi-generator-ignore'), { force: true })

  rmSync(rustTarget, { recursive: true, force: true })
  cpSync(trees.rust, rustTarget, { recursive: true })

  rmSync(clientTarget, { recursive: true, force: true })
  cpSync(trees.client, clientTarget, { recursive: true })
}

function listFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true })
    .flatMap((entry) => {
      const path = join(directory, entry.name)
      return entry.isDirectory()
        ? listFiles(path).map((child) => join(entry.name, child))
        : [entry.name]
    })
    .toSorted()
}

function assertTreeMatches(expected: string, actual: string): void {
  const expectedFiles = listFiles(expected)
  const actualFiles = listFiles(actual)
  const differences = new Set<string>()

  for (const file of expectedFiles) {
    if (!actualFiles.includes(file)) {
      differences.add(`missing ${file}`)
      continue
    }
    if (
      !readFileSync(join(expected, file)).equals(
        readFileSync(join(actual, file)),
      )
    ) {
      differences.add(`changed ${file}`)
    }
  }
  for (const file of actualFiles) {
    if (!expectedFiles.includes(file)) differences.add(`unexpected ${file}`)
  }
  if (differences.size > 0) {
    throw new Error(
      `Generated BrowserOS server API is stale:\n${[...differences].map((line) => `  ${line}`).join('\n')}`,
    )
  }
}

if (import.meta.main) {
  const temporaryRoot = mkdtempSync(join(tmpdir(), 'server-api-codegen-'))
  try {
    const trees = runGenerator(temporaryRoot)
    if (check) {
      assertTreeMatches(
        trees.typescript,
        join(root, 'packages/server-api/src/generated'),
      )
      assertTreeMatches(
        trees.rust,
        join(root, 'crates/server-api/src/generated'),
      )
      assertTreeMatches(
        trees.client,
        join(root, 'packages/server-api-client/src/generated'),
      )
      console.log('BrowserOS server API generated output is current.')
    } else {
      installGenerated(trees)
      console.log(
        'Generated BrowserOS server TypeScript and Rust DTOs plus client operation types.',
      )
    }
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true })
  }
}
