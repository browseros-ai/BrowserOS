import { BaseAIProvider, AIMessage, ChatOptions } from './base';

export class OpenAIProvider extends BaseAIProvider {
  name = 'OpenAI';
  supportsStreaming = true;
  supportsImages = true;

  async chat(
    messages: AIMessage[],
    model: string,
    apiKey: string,
    options?: ChatOptions
  ): Promise<string> {
    const baseUrl = 'https://api.openai.com/v1';
    const url = `${baseUrl}/chat/completions`;

    const body = {
      model,
      messages: messages.map((msg) => ({
        role: msg.role,
        content: msg.content,
      })),
      temperature: options?.temperature ?? 0.7,
      max_tokens: options?.maxTokens ?? 4096,
      top_p: options?.topP ?? 1,
      frequency_penalty: options?.frequencyPenalty ?? 0,
      presence_penalty: options?.presencePenalty ?? 0,
      stream: !!options?.onStream,
    };

    const response = await this.makeRequest(url, apiKey, body, {
      Authorization: `Bearer ${apiKey}`,
    });

    if (options?.onStream) {
      return this.handleStream(
        response,
        options.onStream,
        (line: string) => {
          if (!line.startsWith('data: ')) return null;

          const data = line.slice(6);

          if (data === '[DONE]') {
            return { content: '', done: true };
          }

          try {
            const json = JSON.parse(data);
            const content = json.choices[0]?.delta?.content || '';
            const finishReason = json.choices[0]?.finish_reason;

            return {
              content,
              done: finishReason === 'stop',
            };
          } catch {
            return null;
          }
        }
      );
    } else {
      const data = await response.json();
      return data.choices[0]?.message?.content || '';
    }
  }

  async listModels(apiKey: string): Promise<string[]> {
    const response = await fetch('https://api.openai.com/v1/models', {
      headers: {
        Authorization: `Bearer ${apiKey}`,
      },
    });

    if (!response.ok) {
      throw new Error('Failed to fetch models');
    }

    const data = await response.json();
    return data.data
      .filter((model: any) => model.id.includes('gpt'))
      .map((model: any) => model.id);
  }

  async validateApiKey(apiKey: string): Promise<boolean> {
    try {
      const response = await fetch('https://api.openai.com/v1/models', {
        headers: {
          Authorization: `Bearer ${apiKey}`,
        },
      });
      return response.ok;
    } catch {
      return false;
    }
  }
}
