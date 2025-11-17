export interface AIMessage {
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp?: number;
}

export interface StreamChunk {
  content: string;
  done: boolean;
}

export interface ChatOptions {
  temperature?: number;
  maxTokens?: number;
  topP?: number;
  frequencyPenalty?: number;
  presencePenalty?: number;
  onStream?: (chunk: StreamChunk) => void;
  onError?: (error: Error) => void;
}

export interface AIProviderInterface {
  name: string;
  supportsStreaming: boolean;
  supportsImages: boolean;

  /**
   * Send a chat message and get a response
   */
  chat(
    messages: AIMessage[],
    model: string,
    apiKey: string,
    options?: ChatOptions
  ): Promise<string>;

  /**
   * List available models (if supported)
   */
  listModels?(apiKey: string): Promise<string[]>;

  /**
   * Validate API key
   */
  validateApiKey?(apiKey: string): Promise<boolean>;
}

export abstract class BaseAIProvider implements AIProviderInterface {
  abstract name: string;
  abstract supportsStreaming: boolean;
  abstract supportsImages: boolean;

  abstract chat(
    messages: AIMessage[],
    model: string,
    apiKey: string,
    options?: ChatOptions
  ): Promise<string>;

  /**
   * Helper to handle streaming responses
   */
  protected async handleStream(
    response: Response,
    onStream: (chunk: StreamChunk) => void,
    parseChunk: (line: string) => { content: string; done: boolean } | null
  ): Promise<string> {
    const reader = response.body?.getReader();
    if (!reader) {
      throw new Error('Response body is not readable');
    }

    const decoder = new TextDecoder();
    let fullContent = '';

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        const chunk = decoder.decode(value, { stream: true });
        const lines = chunk.split('\n');

        for (const line of lines) {
          if (!line.trim()) continue;

          const parsed = parseChunk(line);
          if (parsed) {
            fullContent += parsed.content;
            onStream(parsed);

            if (parsed.done) {
              return fullContent;
            }
          }
        }
      }
    } catch (error) {
      throw new Error(`Streaming error: ${error}`);
    }

    return fullContent;
  }

  /**
   * Helper to make API requests
   */
  protected async makeRequest(
    url: string,
    apiKey: string,
    body: any,
    headers: Record<string, string> = {}
  ): Promise<Response> {
    const response = await fetch(url, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...headers,
      },
      body: JSON.stringify(body),
    });

    if (!response.ok) {
      let errorMessage: string;
      try {
        const errorData = await response.json();
        errorMessage = errorData.error?.message || errorData.message || response.statusText;
      } catch {
        errorMessage = response.statusText;
      }
      throw new Error(`API Error (${response.status}): ${errorMessage}`);
    }

    return response;
  }
}
