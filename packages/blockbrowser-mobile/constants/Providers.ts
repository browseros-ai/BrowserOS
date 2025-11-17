export enum ProviderType {
  BLOCKBROWSER = 'blockbrowser',
  OPENAI = 'openai',
  OPENAI_COMPATIBLE = 'openai_compatible',
  ANTHROPIC = 'anthropic',
  GOOGLE_GEMINI = 'google_gemini',
  OLLAMA = 'ollama',
  OPENROUTER = 'openrouter',
  CUSTOM = 'custom',
}

export interface AIProvider {
  id: string;
  name: string;
  type: ProviderType;
  isDefault: boolean;
  isBuiltIn: boolean;
  baseUrl?: string;
  apiKey?: string;
  modelId?: string;
  capabilities?: {
    supportsImages: boolean;
    supportsStreaming: boolean;
  };
  modelConfig?: {
    contextWindow: number;
    temperature: number;
    maxTokens?: number;
  };
  createdAt: number;
  updatedAt: number;
}

export const PROVIDER_DEFAULTS: Record<
  ProviderType,
  Partial<AIProvider>
> = {
  [ProviderType.BLOCKBROWSER]: {
    name: 'BlockBrowser',
    isBuiltIn: true,
    capabilities: {
      supportsImages: true,
      supportsStreaming: true,
    },
  },
  [ProviderType.OPENAI]: {
    name: 'OpenAI',
    baseUrl: 'https://api.openai.com/v1',
    modelId: 'gpt-4o',
    capabilities: {
      supportsImages: true,
      supportsStreaming: true,
    },
    modelConfig: {
      contextWindow: 128000,
      temperature: 0.7,
      maxTokens: 4096,
    },
  },
  [ProviderType.ANTHROPIC]: {
    name: 'Anthropic',
    baseUrl: 'https://api.anthropic.com',
    modelId: 'claude-sonnet-4-20250514',
    capabilities: {
      supportsImages: true,
      supportsStreaming: true,
    },
    modelConfig: {
      contextWindow: 200000,
      temperature: 0.7,
      maxTokens: 4096,
    },
  },
  [ProviderType.GOOGLE_GEMINI]: {
    name: 'Google Gemini',
    baseUrl: 'https://generativelanguage.googleapis.com',
    modelId: 'gemini-2.0-flash-exp',
    capabilities: {
      supportsImages: true,
      supportsStreaming: true,
    },
    modelConfig: {
      contextWindow: 1048576,
      temperature: 0.7,
      maxTokens: 8192,
    },
  },
  [ProviderType.OLLAMA]: {
    name: 'Ollama',
    baseUrl: 'http://127.0.0.1:11434',
    modelId: 'llama2',
    capabilities: {
      supportsImages: false,
      supportsStreaming: true,
    },
    modelConfig: {
      contextWindow: 4096,
      temperature: 0.7,
    },
  },
  [ProviderType.OPENROUTER]: {
    name: 'OpenRouter',
    baseUrl: 'https://openrouter.ai/api/v1',
    modelId: 'openai/gpt-4o',
    capabilities: {
      supportsImages: true,
      supportsStreaming: true,
    },
    modelConfig: {
      contextWindow: 128000,
      temperature: 0.7,
    },
  },
  [ProviderType.OPENAI_COMPATIBLE]: {
    name: 'OpenAI Compatible',
    baseUrl: 'http://127.0.0.1:1234/v1',
    modelId: 'local-model',
    capabilities: {
      supportsImages: false,
      supportsStreaming: true,
    },
    modelConfig: {
      contextWindow: 8192,
      temperature: 0.7,
    },
  },
  [ProviderType.CUSTOM]: {
    name: 'Custom Provider',
    baseUrl: '',
    modelId: '',
    capabilities: {
      supportsImages: false,
      supportsStreaming: false,
    },
    modelConfig: {
      contextWindow: 4096,
      temperature: 0.7,
    },
  },
};

export const PROVIDER_ICONS: Record<ProviderType, string> = {
  [ProviderType.BLOCKBROWSER]: '🅱️',
  [ProviderType.OPENAI]: '🤖',
  [ProviderType.ANTHROPIC]: '🧠',
  [ProviderType.GOOGLE_GEMINI]: '💎',
  [ProviderType.OLLAMA]: '🦙',
  [ProviderType.OPENROUTER]: '🔀',
  [ProviderType.OPENAI_COMPATIBLE]: '⚙️',
  [ProviderType.CUSTOM]: '🔧',
};
