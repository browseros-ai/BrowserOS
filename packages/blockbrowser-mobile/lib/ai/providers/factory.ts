import { AIProviderInterface } from './base';
import { OpenAIProvider } from './openai';
import { AnthropicProvider } from './anthropic';
import { ProviderType } from '@/constants/Providers';

export class AIProviderFactory {
  private static instances: Map<ProviderType, AIProviderInterface> = new Map();

  static getProvider(type: ProviderType): AIProviderInterface {
    // Return cached instance if exists
    if (this.instances.has(type)) {
      return this.instances.get(type)!;
    }

    // Create new instance based on type
    let provider: AIProviderInterface;

    switch (type) {
      case ProviderType.OPENAI:
      case ProviderType.OPENAI_COMPATIBLE:
      case ProviderType.OPENROUTER:
        provider = new OpenAIProvider();
        break;

      case ProviderType.ANTHROPIC:
        provider = new AnthropicProvider();
        break;

      case ProviderType.GOOGLE_GEMINI:
        // For now, use OpenAI-compatible format
        // You can add a specific Gemini provider later
        provider = new OpenAIProvider();
        break;

      case ProviderType.OLLAMA:
      case ProviderType.CUSTOM:
        // Ollama and custom providers use OpenAI-compatible API
        provider = new OpenAIProvider();
        break;

      case ProviderType.BLOCKBROWSER:
      default:
        // BlockBrowser uses OpenAI by default
        provider = new OpenAIProvider();
        break;
    }

    // Cache the instance
    this.instances.set(type, provider);
    return provider;
  }

  static clearCache(): void {
    this.instances.clear();
  }
}
