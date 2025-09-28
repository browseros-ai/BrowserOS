import { MessageType } from '@/lib/types/messaging'
import { PortMessage } from '@/lib/runtime/PortMessaging'
import { Logging } from '@/lib/utils/Logging'

/**
 * Handles LLM provider configuration messages.
 * Since we now only support ChatOpenAI, this handler provides a simplified
 * response with the single provider configuration.
 */
export class ProvidersHandler {
  /**
   * Handle GET_LLM_PROVIDERS message
   * Returns the single ChatOpenAI provider configuration
   */
  async handleGetProviders(
    message: PortMessage,
    port: chrome.runtime.Port
  ): Promise<void> {
    try {
      // Return simplified ChatOpenAI-only configuration
      const config = {
        defaultProviderId: 'openai',
        providers: [{
          id: 'openai',
          name: 'OpenAI',
          type: 'openai',
          isDefault: true,
          isBuiltIn: true,
          apiKey: process.env.OPENAI_API_KEY || '',
          modelId: 'gpt-4o-mini',
          capabilities: { supportsImages: true },
          modelConfig: { contextWindow: 128000, temperature: 0.2 },
          createdAt: new Date().toISOString(),
          updatedAt: new Date().toISOString()
        }]
      }
      
      port.postMessage({
        type: MessageType.WORKFLOW_STATUS,
        payload: { 
          status: 'success', 
          data: { providersConfig: config } 
        },
        id: message.id
      })
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      Logging.log('ProvidersHandler', `Error getting providers: ${errorMessage}`, 'error')
      
      port.postMessage({
        type: MessageType.WORKFLOW_STATUS,
        payload: { 
          status: 'error', 
          error: `Failed to read providers: ${errorMessage}` 
        },
        id: message.id
      })
    }
  }

  /**
   * Handle SAVE_LLM_PROVIDERS message
   * Since we only support ChatOpenAI, this is a no-op
   */
  handleSaveProviders(
    message: PortMessage,
    port: chrome.runtime.Port
  ): void {
    // ChatOpenAI configuration is fixed, no need to save
    Logging.log('ProvidersHandler', 'Provider save requested but ChatOpenAI config is fixed', 'info')
    
    port.postMessage({
      type: MessageType.WORKFLOW_STATUS,
      payload: { status: 'success' },
      id: message.id
    })
  }
}