import type { Provider } from './chatComponentTypes'

export interface ProviderOptionGroup {
  key: 'llm' | 'acp'
  label: string
  options: Provider[]
}

export function groupProviderOptions(
  providers: Provider[],
): ProviderOptionGroup[] {
  const llm = providers.filter((provider) => provider.kind !== 'acp')
  const acp = providers.filter((provider) => provider.kind === 'acp')

  return [
    ...(llm.length
      ? [{ key: 'llm' as const, label: 'AI Providers', options: llm }]
      : []),
    ...(acp.length
      ? [{ key: 'acp' as const, label: 'ACP Models', options: acp }]
      : []),
  ]
}

export function getProviderSearchValue(
  provider: Provider,
  groupLabel: string,
): string {
  return [provider.id, provider.name, provider.type, groupLabel]
    .filter(Boolean)
    .join(' ')
}

export function getProviderSubtitle(provider: Provider): string | undefined {
  if (provider.kind !== 'acp') return undefined
  return provider.modelControl === 'best-effort'
    ? 'ACP model · best effort'
    : 'ACP model'
}
