import { BaseAIProvider, AIMessage, ChatOptions } from './base';

export class AnthropicProvider extends BaseAIProvider {
  name = 'Anthropic';
  supportsStreaming = true;
  supportsImages = true;

  async chat(
    messages: AIMessage[],
    model: string,
    apiKey: string,
    options?: ChatOptions
  ): Promise<string> {
    const baseUrl = 'https://api.anthropic.com/v1';
    const url = `${baseUrl}/messages`;

    // Filter out system messages and convert to Claude format
    const systemMessage = messages.find((m) => m.role === 'system');
    const conversationMessages = messages.filter((m) => m.role !== 'system');

    const body: any = {
      model,
      messages: conversationMessages.map((msg) => ({
        role: msg.role === 'user' ? 'user' : 'assistant',
        content: msg.content,
      })),
      max_tokens: options?.maxTokens ?? 4096,
      temperature: options?.temperature ?? 0.7,
      stream: !!options?.onStream,
    };

    // Add system message if present
    if (systemMessage) {
      body.system = systemMessage.content;
    }

    const response = await this.makeRequest(url, apiKey, body, {
      'x-api-key': apiKey,
      'anthropic-version': '2023-06-01',
    });

    if (options?.onStream) {
      return this.handleStream(
        response,
        options.onStream,
        (line: string) => {
          if (!line.startsWith('data: ')) return null;

          const data = line.slice(6);

          try {
            const json = JSON.parse(data);

            if (json.type === 'content_block_delta') {
              const content = json.delta?.text || '';
              return { content, done: false };
            } else if (json.type === 'message_stop') {
              return { content: '', done: true };
            }

            return null;
          } catch {
            return null;
          }
        }
      );
    } else {
      const data = await response.json();
      return data.content[0]?.text || '';
    }
  }

  async validateApiKey(apiKey: string): Promise<boolean> {
    try {
      const response = await fetch('https://api.anthropic.com/v1/messages', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'x-api-key': apiKey,
          'anthropic-version': '2023-06-01',
        },
        body: JSON.stringify({
          model: 'claude-3-haiku-20240307',
          messages: [{ role: 'user', content: 'Hi' }],
          max_tokens: 1,
        }),
      });

      return response.ok || response.status === 400; // 400 is ok for validation
    } catch {
      return false;
    }
  }
}
