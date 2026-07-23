export interface ProtocolProperty {
  name: string
  description?: string
  optional?: boolean
  type?: string
  $ref?: string
  enum?: string[]
  items?: { type?: string; $ref?: string }
  properties?: ProtocolProperty[]
}

export interface ProtocolType {
  id: string
  description?: string
  type: string
  enum?: string[]
  properties?: ProtocolProperty[]
  items?: { type?: string; $ref?: string }
}

export interface ProtocolCommand {
  name: string
  description?: string
  parameters?: ProtocolProperty[]
  returns?: ProtocolProperty[]
}

export interface ProtocolEvent {
  name: string
  description?: string
  parameters?: ProtocolProperty[]
}

export interface ProtocolDomain {
  domain: string
  description?: string
  dependencies?: string[]
  types?: ProtocolType[]
  commands?: ProtocolCommand[]
  events?: ProtocolEvent[]
}

export interface Protocol {
  version: { major: string; minor: string }
  domains: ProtocolDomain[]
}

export function parseProtocol(source: string): Protocol {
  let data: unknown
  try {
    data = JSON.parse(source)
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    throw new Error(`Invalid protocol JSON: ${message}`)
  }

  if (
    typeof data !== 'object' ||
    data === null ||
    !('version' in data) ||
    !('domains' in data) ||
    !Array.isArray(data.domains)
  ) {
    throw new Error('Invalid protocol JSON: missing version or domains')
  }

  const version = data.version
  if (
    typeof version !== 'object' ||
    version === null ||
    !('major' in version) ||
    typeof version.major !== 'string' ||
    !('minor' in version) ||
    typeof version.minor !== 'string'
  ) {
    throw new Error('Invalid protocol JSON: invalid version')
  }

  const domainNames = new Set<string>()
  for (const domain of data.domains) {
    if (
      typeof domain !== 'object' ||
      domain === null ||
      !('domain' in domain) ||
      typeof domain.domain !== 'string' ||
      domain.domain.length === 0
    ) {
      throw new Error('Invalid protocol JSON: invalid domain')
    }
    if (domainNames.has(domain.domain)) {
      throw new Error(
        `Invalid protocol JSON: duplicate domain ${domain.domain}`,
      )
    }
    domainNames.add(domain.domain)
  }

  return data as Protocol
}
