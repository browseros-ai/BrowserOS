import {
  Anthropic,
  Azure,
  Bedrock,
  Gemini,
  Kimi,
  LmStudio,
  Minimax,
  Ollama,
  OpenAI,
  OpenRouter,
  Qwen,
} from '@lobehub/icons'
import { Bot, Github } from 'lucide-react'
import type { FC, SVGProps } from 'react'
import ProductLogoSvg from '@/assets/product_logo.svg'
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
  browseros: null,
  moonshot: Kimi,
  'chatgpt-pro': OpenAI,
  'github-copilot': Github,
  'qwen-code': Qwen,
  minimax: Minimax,
}

interface ProviderIconProps {
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

/**
 * BrowserOS branded icon component
 * @public
 */
export const BrowserOSIcon: FC<{ size?: number; className?: string }> = ({
  size = 20,
  className,
}) => {
  return (
    <img
      src={ProductLogoSvg}
      alt="BrowserOS"
      width={size}
      height={size}
      className={className}
    />
  )
}
