/**
 * Codegen for the canonical BrowserClaw API. Reads the OpenAPI spec at
 * `contracts/claw-api/openapi.yaml` and emits both generated clients:
 * the TypeScript fetch client into `packages/claw-api/src/generated`
 * and the Rust DTOs into `crates/claw-api/src/generated`. Neither tree
 * is ever hand-edited — change the spec, then regenerate with
 * `bun run codegen:claw-api`.
 *
 * The generator runs in Docker with a pinned image and the output is
 * normalized (trailing whitespace stripped, generated Rust run through
 * rustfmt) so the emitted trees are byte-identical across machines.
 * That determinism is what lets `--check` (`bun run
 * codegen:claw-api:check`, the CI drift gate) compare a fresh
 * generation against the committed trees byte for byte.
 */

import { spawnSync } from 'node:child_process'
import {
  cpSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join, relative } from 'node:path'
import { parse } from 'yaml'

const root = join(import.meta.dir, '../..')
const image = 'openapitools/openapi-generator-cli:v7.22.0'
const check = process.argv.includes('--check')

interface GeneratedTrees {
  typescript: string
  rust: string
}

/**
 * OpenAPI Generator 7.22 guards required headers twice: first by throwing,
 * then by conditionally assigning them. Flatten the unreachable second guard
 * so generated clients remain clean under whole-repository static analysis.
 */
export function flattenRequiredHeaderGuards(source: string): string {
  const requiredParameters = new Set(
    [...source.matchAll(/if \(requestParameters\['([^']+)'\] == null\) \{/g)]
      .map((match) => match[1])
      .filter((parameter): parameter is string => parameter !== undefined),
  )
  let normalized = source
  for (const parameter of requiredParameters) {
    const escaped = parameter.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
    const guardedAssignment = new RegExp(
      `        if \\(requestParameters\\['${escaped}'\\] != null\\) \\{\\n(            headerParameters\\[[^\\n]+\\n)        \\}`,
      'g',
    )
    normalized = normalized.replace(
      guardedAssignment,
      (_match, assignment: string) =>
        assignment.replace(/^ {12}/, '        ').trimEnd(),
    )
  }
  return normalized
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
            /^\.\/schemas\/([a-z][a-z0-9-]*)\.yaml#\/([A-Za-z][A-Za-z0-9]*)$/,
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
  return `${header}\n\n${[...imports].toSorted().join('\n')}\n\n${parsed
    .map(({ source }) => source.body)
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

  return `${preamble}\n\n${importBlock}\n\n${parsed
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
  const importedModel = target.match(/^\.\/([A-Za-z][A-Za-z0-9]*)\.js$/)?.[1]
  const importedGroup = importedModel ? modelGroups[importedModel] : undefined
  if (!importedModel || !importedGroup) {
    throw new Error(
      `TypeScript model ${model} imports unknown model target ${target}`,
    )
  }
  return importedGroup === sourceGroup ? null : `./${importedGroup}.js`
}

export function emitTypeScriptModelIndex(groups: Iterable<string>): string {
  return `/* tslint:disable */\n/* eslint-disable */\n${[...new Set(groups)]
    .toSorted()
    .map((group) => `export * from './${group}.js';`)
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

function parseTypeScriptModel(
  model: string,
  source: string,
): ParsedTypeScriptModel {
  const normalized = source.trimEnd()
  const firstImport = normalized.indexOf('\nimport ')
  if (firstImport === -1) {
    throw new Error(`TypeScript model ${model} has no generated imports`)
  }

  const preamble = normalized.slice(0, firstImport).trimEnd()
  const imports: ParsedTypeScriptImport[] = []
  let cursor = firstImport + 1
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
  if (normalized[cursor] !== '\n') {
    throw new Error(`TypeScript model ${model} has an unrecognized import`)
  }

  return {
    preamble,
    imports,
    body: normalized.slice(cursor + 1).trim(),
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

function runGenerator(outputRoot: string): GeneratedTrees {
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
    '/local/contracts/claw-api/openapi.yaml',
  ]

  runDocker([
    ...common,
    '-g',
    'typescript-fetch',
    '-o',
    '/out/typescript',
    '--global-property',
    'apiDocs=false,modelDocs=false,apiTests=false,modelTests=false',
    '--additional-properties',
    'supportsES6=true,typescriptThreePlus=true,importFileExtension=.js,disallowAdditionalPropertiesIfNotPresent=false',
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
    'packageName=claw-api,packageVersion=1.0.0',
  ])

  const typescript = join(outputRoot, 'typescript')
  const rust = join(outputRoot, 'rust/src/models')
  rmSync(join(typescript, '.openapi-generator'), {
    recursive: true,
    force: true,
  })
  rmSync(join(typescript, '.openapi-generator-ignore'), { force: true })
  for (const file of listFiles(typescript).filter((path) =>
    path.endsWith('.ts'),
  )) {
    const path = join(typescript, file)
    const source = readFileSync(path, 'utf8')
    let normalized = `${source
      .split('\n')
      .map((line) => line.trimEnd())
      .join('\n')
      .trimEnd()}\n`
    normalized = flattenRequiredHeaderGuards(normalized)
    if (file === 'runtime.ts') {
      // TypeScript requires `override` for the inherited Error.cause property;
      // OpenAPI Generator 7.22's FetchError template predates that check.
      const generatorConstructor =
        'constructor(public cause: Error, msg?: string)'
      if (!normalized.includes(generatorConstructor)) {
        throw new Error('OpenAPI Generator FetchError template changed')
      }
      normalized = normalized.replace(
        generatorConstructor,
        'constructor(public override cause: Error, msg?: string)',
      )
    }
    writeFileSync(path, normalized)
  }
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
  }
}

function runDocker(args: string[]): void {
  const result = spawnSync('docker', args, { stdio: 'inherit' })
  if (result.error) throw result.error
  if (result.status !== 0) {
    throw new Error(`OpenAPI Generator exited with status ${result.status}`)
  }
}

function installGenerated(trees: GeneratedTrees): void {
  const typescriptTarget = join(root, 'packages/claw-api/src/generated')
  const rustTarget = join(root, 'crates/claw-api/src/generated')

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
      `Generated BrowserClaw API is stale:\n${[...differences].map((line) => `  ${line}`).join('\n')}`,
    )
  }
}

if (import.meta.main) {
  const temporaryRoot = mkdtempSync(join(tmpdir(), 'claw-api-codegen-'))
  try {
    const trees = runGenerator(temporaryRoot)
    if (check) {
      assertTreeMatches(
        trees.typescript,
        join(root, 'packages/claw-api/src/generated'),
      )
      assertTreeMatches(trees.rust, join(root, 'crates/claw-api/src/generated'))
      console.log('BrowserClaw API generated output is current.')
    } else {
      installGenerated(trees)
      console.log('Generated BrowserClaw TypeScript client and Rust DTOs.')
    }
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true })
  }
}
