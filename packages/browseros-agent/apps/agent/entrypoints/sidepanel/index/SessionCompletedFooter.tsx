import type { FC } from 'react'
import type { Provider } from '@/components/chat/chatComponentTypes'

export function formatSessionCompletedLabel(
  provider: Provider | undefined,
): string | null {
  if (!provider) return null
  if (provider.kind === 'acp') {
    if (provider.adapterName && provider.modelLabel) {
      return `${provider.adapterName} · ${provider.modelLabel}`
    }
    return provider.adapterName ?? provider.modelLabel ?? provider.name
  }
  return provider.name
}

interface SessionCompletedFooterProps {
  provider: Provider | undefined
}

export const SessionCompletedFooter: FC<SessionCompletedFooterProps> = ({
  provider,
}) => {
  const label = formatSessionCompletedLabel(provider)
  if (!label) return null

  return (
    <div className="flex justify-center px-3 pt-1 pb-2">
      <span className="text-muted-foreground/70 text-xs">
        Session completed with{' '}
        <span className="font-medium text-muted-foreground">{label}</span>
      </span>
    </div>
  )
}
