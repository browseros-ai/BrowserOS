/**
 * Build-time examples from the prelude's actual input types. The compiler stays outside the
 * QuickJS bundle; its option tuples plus each input's optional fields define one logical key
 * set whether a family's options still live in a bag or have moved to the input itself.
 */
import { resolve } from 'node:path'
import type { SourceFile } from 'typescript/unstable/ast'
import type {
  Checker,
  Symbol as CompilerSymbol,
  Type,
} from 'typescript/unstable/async'

const packageRoot = resolve(import.meta.dir, '..')
const addressing = new Set(['target', 'strict', 'page', 'frame', 'call'])

type Enums = Pick<
  typeof import('typescript/unstable/async'),
  'SymbolFlags' | 'TypeFlags'
>
type Field = { types: Type[]; optional: boolean }

/** Logical key coverage and call samples derived together in one compiler pass. */
export type InputExamples = {
  optionalKeys: Record<string, string[]>
  samples: Record<string, Record<string, unknown>>
}

/** Resolves aliases/unions with the same checker used by tsc, without executing the prelude. */
class Sampler {
  constructor(
    readonly checker: Checker,
    readonly enums: Enums,
  ) {}

  /** Follow the public export whether it is declared locally or re-exported from generated types. */
  async exported(file: SourceFile, name: string): Promise<CompilerSymbol> {
    const module = await this.checker.getSymbolAtLocation(file)
    const symbol = module
      ? (await this.checker.getExportsOfModule(module)).find(
          (entry) => entry.name === name,
        )
      : undefined
    if (!symbol) throw new Error(`Missing input export ${name}`)
    return symbol.flags & this.enums.SymbolFlags.Alias
      ? this.checker.getAliasedSymbol(symbol)
      : symbol
  }

  async property(symbol: CompilerSymbol): Promise<Type> {
    const type = await this.checker.getTypeOfSymbol(symbol)
    if (!type) throw new Error(`No input type for ${symbol.name}`)
    return type
  }

  async defined(type: Type): Promise<Type> {
    const defined = await this.checker.getNonNullableType(type)
    if (!defined || defined.isErrorType())
      throw new Error(
        `Unresolved input type: ${await this.checker.typeToString(type)}`,
      )
    return defined
  }

  /** A field present in only some union arms is optional for the input as a whole. */
  async fields(type: Type): Promise<Map<string, Field>> {
    const defined = await this.defined(type)
    const arms = defined.isUnionType() ? await defined.getTypes() : [defined]
    const found = new Map<
      string,
      { types: Type[]; optional: boolean; arms: number }
    >()
    for (const arm of arms) {
      for (const property of await this.checker.getPropertiesOfType(arm)) {
        const prior = found.get(property.name) ?? {
          types: [],
          optional: false,
          arms: 0,
        }
        prior.types.push(await this.property(property))
        prior.optional ||=
          (property.flags & this.enums.SymbolFlags.Optional) !== 0
        prior.arms++
        found.set(property.name, prior)
      }
    }
    return new Map(
      [...found].map(([name, field]) => [
        name,
        {
          types: field.types,
          optional: field.optional || field.arms < arms.length,
        },
      ]),
    )
  }

  /** One deterministic JSON value per type; method-specific public values are adapted in the script. */
  async value(type: Type, depth = 0): Promise<unknown> {
    const current = await this.defined(type)
    const flags = this.enums.TypeFlags
    if (depth > 8)
      throw new Error(
        `Recursive example type: ${await this.checker.typeToString(current)}`,
      )
    if (
      current.isStringLiteralType() ||
      current.isNumberLiteralType() ||
      current.isBooleanLiteralType()
    )
      return current.value
    if (current.isUnionType()) {
      const types = await current.getTypes()
      const arm =
        types.find((part) => part.isBooleanLiteralType() && !part.value) ??
        types.find((part) => (part.flags & flags.Number) !== 0) ??
        types[0]
      if (!arm) throw new Error('An input union has no example')
      return this.value(arm, depth + 1)
    }
    if (current.flags & flags.String) return 'sample'
    if (current.flags & flags.Number) return 1
    if (current.flags & flags.Boolean) return false
    if (current.flags & (flags.Unknown | flags.Any)) return {}
    if (
      (await this.checker.isArrayLikeType(current)) &&
      current.isTypeReference()
    ) {
      const elements = await this.checker.getTypeArguments(current)
      return Promise.all(
        elements.map((element) => this.value(element, depth + 1)),
      )
    }
    const fields = await this.fields(current)
    if (fields.size)
      return Object.fromEntries(
        await Promise.all(
          [...fields].map(async ([name, field]) => {
            const first = field.types[0]
            if (!first) throw new Error(`No example type for ${name}`)
            return [name, await this.value(first, depth + 1)]
          }),
        ),
      )
    const index = (await this.checker.getIndexInfosOfType(current))[0]
    if (index) return { sample: await this.value(index.valueType, depth + 1) }
    throw new Error(
      `No JSON example for ${await this.checker.typeToString(current)}`,
    )
  }

  async keys(type: Type): Promise<string[]> {
    if (!type.isTypeReference())
      throw new Error('OPTION_KEYS must contain literal tuples')
    return (await this.checker.getTypeArguments(type)).map((key) => {
      if (!key.isStringLiteralType())
        throw new Error('An option key is not a string literal')
      return key.value
    })
  }
}

let cached: Promise<InputExamples> | undefined

/**
 * Load the compiler only when recording starts, after build.ts has checked the Bun version.
 * Its native process is closed before returning; builds and tests never leave a server behind.
 */
export function inputExamples(): Promise<InputExamples> {
  cached ??= reflectInputs()
  return cached
}

async function reflectInputs(): Promise<InputExamples> {
  const native = await import('typescript/unstable/async')
  const api = new native.API({ cwd: packageRoot })
  try {
    const config = resolve(packageRoot, 'tsconfig.json')
    const snapshot = await api.updateSnapshot({ openProjects: [config] })
    const project = snapshot.getProject(config)
    if (!project) throw new Error('The prelude compiler project did not load')
    const wire = await project.program.getSourceFile(
      resolve(packageRoot, 'src/prelude/wire.ts'),
    )
    const generated = await project.program.getSourceFile(
      resolve(packageRoot, 'src/prelude/generated/index.ts'),
    )
    if (!wire || !generated)
      throw new Error('Missing prelude input declarations')
    const sampler = new Sampler(project.checker, native)
    const inputs = await sampler.exported(wire, 'OpInputs')
    const options = await sampler.exported(generated, 'OPTION_KEYS')
    const optionMap = await sampler.fields(await sampler.property(options))
    const optionalKeys: InputExamples['optionalKeys'] = {}
    const samples: InputExamples['samples'] = {}
    for (const op of await project.checker.getPropertiesOfType(
      await project.checker.getDeclaredTypeOfSymbol(inputs),
    )) {
      const fields = await sampler.fields(await sampler.property(op))
      const tuple = optionMap.get(op.name)?.types[0]
      const optionKeys = tuple ? await sampler.keys(tuple) : []
      const optionsType = fields.get('options')?.types[0]
      const bag =
        optionsType &&
        !(await project.checker.isArrayLikeType(
          await sampler.defined(optionsType),
        ))
          ? await sampler.fields(optionsType)
          : undefined
      const nested =
        bag && optionKeys.length > 0 && optionKeys.every((key) => bag.has(key))
      const keys = new Set(optionKeys)
      for (const [name, field] of fields)
        if (
          field.optional &&
          !addressing.has(name) &&
          !(nested && name === 'options')
        )
          keys.add(name)
      optionalKeys[op.name] = [...keys]
        .filter((key) => !addressing.has(key))
        .sort()
      samples[op.name] = Object.fromEntries(
        await Promise.all(
          optionalKeys[op.name].map(async (key) => {
            const field =
              nested && optionKeys.includes(key)
                ? bag.get(key)
                : fields.get(key)
            const type = field?.types[0]
            if (!type) throw new Error(`${op.name}.${key} has no example type`)
            return [key, await sampler.value(type)]
          }),
        ),
      )
    }
    return { optionalKeys, samples }
  } finally {
    await api.close()
  }
}
