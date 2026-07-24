import { createHash } from 'node:crypto'
import { mkdirSync, rmSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { emitDomainApiFile } from './lib/domain-api-emitter'
import { emitDomainFile } from './lib/domain-emitter'
import { domainToKebab } from './lib/naming'
import {
  emitCreateApiFile,
  emitProtocolApiFile,
} from './lib/protocol-api-emitter'
import { parseProtocol } from './lib/protocol-parser'

function requireEnv(name: string): string {
  const value = process.env[name]
  if (!value) {
    console.error(
      `Set ${name} to Chromium's generated combined devtools_protocol/protocol.json`,
    )
    process.exit(1)
  }
  return value
}

const OUT_DIR = join(import.meta.dir, '../../packages/cdp-protocol')
const GEN_DIR = join(OUT_DIR, 'src/generated')
const DOMAINS_DIR = join(GEN_DIR, 'domains')
const DOMAIN_APIS_DIR = join(GEN_DIR, 'domain-apis')

async function main() {
  const protocolPath = resolve(requireEnv('CDP_PROTOCOL_JSON'))
  console.log(`CDP protocol source: ${protocolPath}`)

  let source: string
  try {
    source = await Bun.file(protocolPath).text()
  } catch (error) {
    throw new Error(
      `Failed to read CDP protocol source ${protocolPath}: ${errorMessage(error)}`,
    )
  }

  const sourceHash = createHash('sha256').update(source).digest('hex')
  console.log(`CDP protocol SHA-256: ${sourceHash}`)

  let protocol: ReturnType<typeof parseProtocol>
  try {
    protocol = parseProtocol(source)
  } catch (error) {
    throw new Error(
      `Failed to parse CDP protocol source ${protocolPath}: ${errorMessage(error)}`,
    )
  }
  console.log(
    `Found ${protocol.domains.length} domains (v${protocol.version.major}.${protocol.version.minor})`,
  )

  rmSync(GEN_DIR, { recursive: true, force: true })
  mkdirSync(DOMAINS_DIR, { recursive: true })
  mkdirSync(DOMAIN_APIS_DIR, { recursive: true })

  console.log('Generating domain type files...')
  for (const domain of protocol.domains) {
    const fileName = `${domainToKebab(domain.domain)}.ts`
    const content = emitDomainFile(domain)
    await Bun.write(join(DOMAINS_DIR, fileName), content)
  }

  console.log('Generating domain API files...')
  for (const domain of protocol.domains) {
    const fileName = `${domainToKebab(domain.domain)}.ts`
    const content = emitDomainApiFile(domain)
    await Bun.write(join(DOMAIN_APIS_DIR, fileName), content)
  }

  console.log('Generating protocol-api.ts...')
  await Bun.write(
    join(GEN_DIR, 'protocol-api.ts'),
    emitProtocolApiFile(protocol.domains),
  )

  console.log('Generating create-api.ts...')
  await Bun.write(
    join(GEN_DIR, 'create-api.ts'),
    emitCreateApiFile(protocol.domains),
  )

  console.log('Generating package.json exports...')
  await writePackageJson(protocol)

  console.log('Formatting generated files...')
  const format = Bun.spawnSync(
    [
      process.execPath,
      'x',
      '@biomejs/biome',
      'check',
      '--write',
      '--no-errors-on-unmatched',
      '--files-ignore-unknown=true',
      '--colors=off',
      GEN_DIR,
      join(OUT_DIR, 'package.json'),
    ],
    {
      cwd: join(import.meta.dir, '../..'),
      stdout: 'inherit',
      stderr: 'inherit',
    },
  )
  if (format.exitCode !== 0) {
    throw new Error(`Generated file formatting exited ${format.exitCode}`)
  }

  console.log('Done!')
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

async function writePackageJson(protocol: ReturnType<typeof parseProtocol>) {
  const exports: Record<string, { types: string; default: string }> = {}

  for (const domain of protocol.domains) {
    const kebab = domainToKebab(domain.domain)
    const domainPath = `./src/generated/domains/${kebab}.ts`
    exports[`./domains/${kebab}`] = { types: domainPath, default: domainPath }

    const apiPath = `./src/generated/domain-apis/${kebab}.ts`
    exports[`./domain-apis/${kebab}`] = { types: apiPath, default: apiPath }
  }

  exports['./protocol-api'] = {
    types: './src/generated/protocol-api.ts',
    default: './src/generated/protocol-api.ts',
  }
  exports['./create-api'] = {
    types: './src/generated/create-api.ts',
    default: './src/generated/create-api.ts',
  }

  const pkg = {
    name: '@browseros/cdp-protocol',
    version: '0.0.1',
    type: 'module',
    scripts: {
      typecheck: 'tsc --noEmit',
    },
    exports,
  }

  await Bun.write(
    join(OUT_DIR, 'package.json'),
    `${JSON.stringify(pkg, null, 2)}\n`,
  )
}

main().catch((err) => {
  console.error(`CDP codegen failed: ${errorMessage(err)}`)
  process.exit(1)
})
