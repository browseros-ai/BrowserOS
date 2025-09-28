/**
 * LangChainProvider - ChatOpenAI provider for LLM instance management
 * 
 * This module exports a pre-initialized singleton instance that creates
 * ChatOpenAI LLM instances using LangChain's ChatOpenAI.
 * 
 * Usage: import { getLLM } from '@/lib/llm/LangChainProvider'
 * No manual initialization needed - the singleton is created automatically.
 */
import { ChatOpenAI } from "@langchain/openai"
import { BaseChatModel } from "@langchain/core/language_models/chat_models"
import { BaseMessage } from "@langchain/core/messages"
import { Logging } from '@/lib/utils/Logging'

// Default constants
const DEFAULT_MODEL = "nemo-llm"
const DEFAULT_BASE_URL = "http://localhost:4000"
const DEFAULT_TEMPERATURE = 0.2
const DEFAULT_STREAMING = true
const DEFAULT_MAX_TOKENS = 4096
const DEFAULT_CONTEXT_WINDOW = 128_000  // OpenAI models have 128K context window

// Model capabilities interface
export interface ModelCapabilities {
  maxTokens: number;  // Maximum context window size
  supportsImages: boolean;  // Whether the provider supports image inputs
}

export class LangChainProvider {
  private static instance: LangChainProvider
  
  // Skip token counting flag - set to true for maximum speed (returns fixed estimates)
  private static readonly SKIP_TOKEN_COUNTING = false
  
  // Constructor and initialization
  static getInstance(): LangChainProvider {
    if (!LangChainProvider.instance) {
      LangChainProvider.instance = new LangChainProvider()
    }
    return LangChainProvider.instance
  }
  
  // Public getter methods
  async getLLM(options?: { temperature?: number; maxTokens?: number }): Promise<BaseChatModel> {
    // Create new LLM instance for ChatOpenAI
    Logging.log('LangChainProvider', 'Creating new ChatOpenAI LLM', 'info')
    const llm = this._createChatOpenAI(options)
    
    // Log metrics about the LLM configuration
    const maxTokens = this._calculateMaxTokens(options?.maxTokens)
    await Logging.logMetric('llm.created', {
      provider: 'OpenAI',
      provider_type: 'openai',
      model_name: DEFAULT_MODEL,
      max_tokens: maxTokens,
      temperature: options?.temperature ?? DEFAULT_TEMPERATURE,
    })
    
    return llm
  }
  
  // Get model capabilities for ChatOpenAI
  async getModelCapabilities(): Promise<ModelCapabilities> {
    return { 
      maxTokens: DEFAULT_CONTEXT_WINDOW, 
      supportsImages: true  // OpenAI models support images
    }
  }
  
  clearCache(): void {
    // No cache to clear in simplified version
  }
  
  /**
   * Patches token counting methods on any chat model for ultra-fast approximation.
   * This eliminates tiktoken "Unknown model" errors and maximizes performance.
   * Uses bit shift operations for speed: 4 chars ≈ 1 token
   */
  private _patchTokenCounting<T extends BaseChatModel>(model: T): T {
    const _CHARS_PER_TOKEN = 2  // Bit shift for division by 4: x >> 2
    const _MESSAGE_OVERHEAD = 20      // Estimated chars for message structure (role, formatting)
    const _COMPLEX_CONTENT_ESTIMATE = 100  // Rough char estimate for non-string content
    
    // Cast model to any for monkey-patching
    const m = model as any
    
    // Ultra-fast mode: skip counting entirely for maximum performance
    if (LangChainProvider.SKIP_TOKEN_COUNTING) {
      m.getNumTokens = async () => 100 
      m.getNumTokensFromMessages = async () => 5000 
      return model
    }
    
    // Fast approximation for single text strings using bit shift
    m.getNumTokens = async function(text: string): Promise<number> {
      // Add 3 before shift for ceiling division: (x + 3) >> 2 ≈ Math.ceil(x / 4)
      // This is ~2-3x faster than Math.ceil(x / 4)
      return (text.length + 3) >> _CHARS_PER_TOKEN
    }
    
    // Optimized token counting for message arrays
    m.getNumTokensFromMessages = async function(messages: BaseMessage[]): Promise<number> {
      // Pre-calculate total overhead for all messages (faster than per-message addition)
      let totalChars = messages.length * _MESSAGE_OVERHEAD
      
      for (const msg of messages) {
        const content = (msg as any).content
        
        if (typeof content === 'string') {
          totalChars += content.length
          continue  // Skip remaining checks for speed
        }
        
        // Handle complex content without expensive JSON.stringify
        if (Array.isArray(content)) {
          // Use bit shift for multiplication: << 6 is multiply by 64
          // Slightly overestimate to avoid JSON.stringify cost
          totalChars += content.length << 6  
        } else if (content) {
          // Fixed estimate for other content types
          totalChars += _COMPLEX_CONTENT_ESTIMATE
        }
        // Note: Skipping name and additional_kwargs for speed
        // These are rare and have minimal impact on token count
      }
      
      // Use bit shift for final division with ceiling
      return (totalChars + 3) >> _CHARS_PER_TOKEN
    }
    
    return model
  }
  
  /**
   * Calculate appropriate maxTokens based on user request and context window
   * @param requestedMaxTokens - User-requested max tokens (optional)
   * @returns Calculated max tokens for the response
   */
  private _calculateMaxTokens(requestedMaxTokens?: number): number {
    if (requestedMaxTokens) {
      // User explicitly requested a limit - respect it but cap at context window
      return Math.min(requestedMaxTokens, DEFAULT_CONTEXT_WINDOW)
    } else {
      // No explicit request - use reasonable default capped by 50% of context window
      // This leaves room for input and conversation history
      return Math.min(DEFAULT_MAX_TOKENS, Math.floor(DEFAULT_CONTEXT_WINDOW * 0.5))
    }
  }
  
  // ChatOpenAI provider
  private _createChatOpenAI(options?: { temperature?: number; maxTokens?: number }): ChatOpenAI {
    const apiKey = process.env.OPENAI_API_KEY
    if (!apiKey) {
      Logging.log('LangChainProvider', 
        'Warning: No OPENAI_API_KEY environment variable found, using default', 
        'warning')
    }
    
    const temperature = options?.temperature ?? DEFAULT_TEMPERATURE
    const maxTokens = this._calculateMaxTokens(options?.maxTokens)
    
    const config: any = {
      modelName: DEFAULT_MODEL,
      streaming: DEFAULT_STREAMING,
      openAIApiKey: apiKey,
      temperature,
      maxTokens,
      configuration: {
        baseURL: DEFAULT_BASE_URL, // LiteLLM proxy
      },
    }
    
    const model = new ChatOpenAI(config)
    return this._patchTokenCounting(model)
  }
}

// Export singleton instance for easy access
export const langChainProvider = LangChainProvider.getInstance()

// Convenience function for quick access
export async function getLLM(options?: { temperature?: number; maxTokens?: number }): Promise<BaseChatModel> {
  return langChainProvider.getLLM(options)
}
