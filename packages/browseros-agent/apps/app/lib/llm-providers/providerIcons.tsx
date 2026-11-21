import {
  Anthropic,
  Azure,
  Bedrock,
  Fireworks,
  Gemini,
  Kimi,
  LmStudio,
  Ollama,
  OpenAI,
  OpenRouter,
} from '@lobehub/icons'
import { Bot } from 'lucide-react'
import type { FC, SVGProps } from 'react'
import type { ProviderType } from './types'

interface IconProps extends SVGProps<SVGSVGElement> {
  size?: number | string
}

type IconComponent = FC<IconProps>

const providerIconMap: Record<ProviderType, IconComponent | null> = {
  anthropic: Anthropic,
  openai: OpenAI,
  'openai-compatible': OpenAI,
  google: Gemini,
  openrouter: OpenRouter,
  azure: Azure,
  ollama: Ollama,
  lmstudio: LmStudio,
  bedrock: Bedrock,
  moonshot: Kimi,
  'chatgpt-pro': OpenAI,
  fireworks: Fireworks,
}

export interface ProviderIconProps {
  type: ProviderType
  size?: number
  className?: string
}

/**
 * Provider icon component that renders the appropriate icon for each provider type
 * @public
 */
export const ProviderIcon: FC<ProviderIconProps> = ({
  type,
  size = 20,
  className,
}) => {
  const IconComponent = providerIconMap[type]

  if (IconComponent) {
    return <IconComponent size={size} className={className} />
  }

  return <Bot size={size} className={className} />
}
