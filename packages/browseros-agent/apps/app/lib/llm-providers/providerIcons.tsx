import {
  Anthropic,
  Azure,
  Bedrock,
  Gemini,
  Kimi,
  LmStudio,
  Ollama,
  OpenAI,
  OpenRouter,
  Qwen,
} from '@lobehub/icons'
import { Bot, Github, Sparkles } from 'lucide-react'
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
  codex: OpenAI,
  'claude-code': Anthropic,
  opencode: null,
  'opencode-go': null,
  'opencode-zen': null,
  'acp-custom': null,
  'remote-hermes': Sparkles,
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
  if (type === 'opencode' || type === 'opencode-go' || type === 'opencode-zen') {
    return <RequestBrowserIcon size={size} className={className} />
  }

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
  return <RequestBrowserIcon size={size} className={className} />
}

/**
 * Request Browser product icon used for the local assistant and OpenCode-backed
 * provider. Keeping this as a shared component prevents the assistant header,
 * provider picker, and settings surfaces from drifting into different brands.
 */
export const RequestBrowserIcon: FC<{ size?: number; className?: string }> = ({
  size = 20,
  className,
}) => {
  return (
    <img
      src={ProductLogoSvg}
      alt="Request Browser"
      width={size}
      height={size}
      className={className}
    />
  )
}
