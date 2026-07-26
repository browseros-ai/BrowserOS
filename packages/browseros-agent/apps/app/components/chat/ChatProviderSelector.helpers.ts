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
      ? [
          {
            key: 'llm' as const,
            label: 'BrowserOS agent + your LLM',
            options: llm,
          },
        ]
      : []),
    ...(acp.length
      ? [{ key: 'acp' as const, label: '3p agents', options: acp }]
      : []),
  ]
}

export function getProviderSearchValue(
  provider: Provider,
  groupLabel: string,
): string {
  return [
    provider.id,
    provider.name,
    provider.type,
    groupLabel,
    provider.adapterName,
    provider.modelLabel,
  ]
    .filter(Boolean)
    .join(' ')
}

export function getProviderSubtitle(provider: Provider): string | undefined {
  if (provider.kind !== 'acp') return undefined
  return [
    provider.adapterName,
    provider.modelLabel,
    provider.modelControl === 'best-effort' ? 'best effort' : undefined,
  ]
    .filter(Boolean)
    .join(' · ')
}
