import * as SecureStore from 'expo-secure-store';

export class SecureStorage {
  private static readonly KEY_PREFIX = 'blockbrowser_';

  /**
   * Save an API key securely
   */
  static async saveApiKey(provider: string, key: string): Promise<void> {
    try {
      const storageKey = `${this.KEY_PREFIX}${provider}_api_key`;
      await SecureStore.setItemAsync(storageKey, key);
    } catch (error) {
      console.error('Failed to save API key:', error);
      throw new Error(`Failed to save API key for ${provider}`);
    }
  }

  /**
   * Retrieve an API key
   */
  static async getApiKey(provider: string): Promise<string | null> {
    try {
      const storageKey = `${this.KEY_PREFIX}${provider}_api_key`;
      return await SecureStore.getItemAsync(storageKey);
    } catch (error) {
      console.error('Failed to retrieve API key:', error);
      return null;
    }
  }

  /**
   * Delete an API key
   */
  static async deleteApiKey(provider: string): Promise<void> {
    try {
      const storageKey = `${this.KEY_PREFIX}${provider}_api_key`;
      await SecureStore.deleteItemAsync(storageKey);
    } catch (error) {
      console.error('Failed to delete API key:', error);
    }
  }

  /**
   * Check if an API key exists
   */
  static async hasApiKey(provider: string): Promise<boolean> {
    const key = await this.getApiKey(provider);
    return key !== null && key.length > 0;
  }

  /**
   * Save any secure value
   */
  static async saveValue(key: string, value: string): Promise<void> {
    try {
      const storageKey = `${this.KEY_PREFIX}${key}`;
      await SecureStore.setItemAsync(storageKey, value);
    } catch (error) {
      console.error('Failed to save value:', error);
      throw new Error(`Failed to save ${key}`);
    }
  }

  /**
   * Get any secure value
   */
  static async getValue(key: string): Promise<string | null> {
    try {
      const storageKey = `${this.KEY_PREFIX}${key}`;
      return await SecureStore.getItemAsync(storageKey);
    } catch (error) {
      console.error('Failed to retrieve value:', error);
      return null;
    }
  }

  /**
   * Delete any secure value
   */
  static async deleteValue(key: string): Promise<void> {
    try {
      const storageKey = `${this.KEY_PREFIX}${key}`;
      await SecureStore.deleteItemAsync(storageKey);
    } catch (error) {
      console.error('Failed to delete value:', error);
    }
  }

  /**
   * Clear all secure storage (use with caution!)
   */
  static async clearAll(): Promise<void> {
    // Note: SecureStore doesn't provide a way to list all keys
    // So we'll need to manually delete known keys
    const providers = [
      'openai',
      'anthropic',
      'google_gemini',
      'ollama',
      'openrouter',
      'custom',
    ];

    for (const provider of providers) {
      await this.deleteApiKey(provider);
    }
  }
}
