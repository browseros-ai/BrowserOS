import { domainToKebab } from './naming'
import type { ProtocolDomain } from './protocol-parser'

export function emitProtocolApiFile(domains: ProtocolDomain[]): string {
  const lines: string[] = []

  const sorted = [...domains].sort((a, b) => {
    const aPath = domainToKebab(a.domain)
    const bPath = domainToKebab(b.domain)
    if (aPath < bPath) return -1
    if (aPath > bPath) return 1
    return 0
  })

  lines.push('// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──')
  lines.push('')

  for (const domain of sorted) {
    const kebab = domainToKebab(domain.domain)
    lines.push(
      `import type { ${domain.domain}Api } from './domain-apis/${kebab}'`,
    )
  }

  lines.push('')
  lines.push('export interface ProtocolApi {')

  for (const domain of domains) {
    lines.push(`  readonly ${domain.domain}: ${domain.domain}Api`)
  }

  lines.push('}')
  lines.push('')

  return lines.join('\n')
}

export function emitCreateApiFile(domains: ProtocolDomain[]): string {
  const lines: string[] = []

  lines.push('// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──')
  lines.push('')
  lines.push("import type { ProtocolApi } from './protocol-api'")
  lines.push('')
  lines.push('export type RawSend = (')
  lines.push('  method: string,')
  lines.push('  params?: Record<string, unknown>,')
  lines.push(') => Promise<unknown>')
  lines.push('')
  lines.push('export type RawOn = (')
  lines.push('  event: string,')
  lines.push('  handler: (params: unknown) => void,')
  lines.push(') => () => void')
  lines.push('')
  lines.push(
    'function createDomainProxy(domain: string, send: RawSend, on: RawOn): unknown {',
  )
  lines.push('  return new Proxy(Object.create(null), {')
  lines.push('    get(_, method: string) {')
  lines.push("      if (method === 'on') {")
  lines.push(
    '        return (event: string, handler: (params: unknown) => void) =>',
  )
  // biome-ignore lint/suspicious/noTemplateCurlyInString: emitting template literal source code
  lines.push('          on(`${domain}.${event}`, handler)')
  lines.push('      }')
  lines.push('      return (params?: Record<string, unknown>) =>')
  // biome-ignore lint/suspicious/noTemplateCurlyInString: emitting template literal source code
  lines.push('        send(`${domain}.${method}`, params)')
  lines.push('    },')
  lines.push('  })')
  lines.push('}')
  lines.push('')
  lines.push(
    'export function createProtocolApi(send: RawSend, on: RawOn): ProtocolApi {',
  )
  lines.push('  return {')

  for (const domain of domains) {
    lines.push(
      `    ${domain.domain}: createDomainProxy('${domain.domain}', send, on),`,
    )
  }

  lines.push('  } as unknown as ProtocolApi')
  lines.push('}')
  lines.push('')

  return lines.join('\n')
}
