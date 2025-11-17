import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { ProviderType } from '@/constants/Providers';

export interface AIProvider {
  id: string;
  name: string;
  type: ProviderType;
  enabled: boolean;
  models: string[];
  baseUrl?: string;
}

interface SettingsState {
  // AI Providers
  aiProviders: AIProvider[];
  defaultProviderId: string;

  // Browser Settings
  enableJavaScript: boolean;
  blockAds: boolean;
  blockTrackers: boolean;
  clearOnExit: boolean;

  // Provider Actions
  updateProvider: (id: string, updates: Partial<AIProvider>) => void;
  setBlockAds: (value: boolean) => void;
  setBlockTrackers: (value: boolean) => void;
  setEnableJavaScript: (value: boolean) => void;
  setClearOnExit: (value: boolean) => void;
}

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      aiProviders: [
        {
          id: 'blockbrowser',
          name: 'BlockBrowser',
          type: ProviderType.BLOCKBROWSER,
          enabled: true,
          models: ['gpt-4'],
        },
        {
          id: 'openai',
          name: 'OpenAI',
          type: ProviderType.OPENAI,
          enabled: false,
          models: ['gpt-4', 'gpt-3.5-turbo'],
          baseUrl: 'https://api.openai.com/v1',
        },
        {
          id: 'anthropic',
          name: 'Anthropic',
          type: ProviderType.ANTHROPIC,
          enabled: false,
          models: ['claude-3-opus', 'claude-3-sonnet'],
          baseUrl: 'https://api.anthropic.com',
        },
        {
          id: 'google',
          name: 'Google Gemini',
          type: ProviderType.GOOGLE_GEMINI,
          enabled: false,
          models: ['gemini-pro'],
          baseUrl: 'https://generativelanguage.googleapis.com',
        },
        {
          id: 'openrouter',
          name: 'OpenRouter',
          type: ProviderType.OPENROUTER,
          enabled: false,
          models: ['openai/gpt-4'],
          baseUrl: 'https://openrouter.ai/api/v1',
        },
        {
          id: 'ollama',
          name: 'Ollama',
          type: ProviderType.OLLAMA,
          enabled: false,
          models: ['llama2', 'mistral'],
          baseUrl: 'http://127.0.0.1:11434',
        },
      ],
      defaultProviderId: 'blockbrowser',
      enableJavaScript: true,
      blockAds: false,
      blockTrackers: false,
      clearOnExit: false,

      updateProvider: (id: string, updates: Partial<AIProvider>) => {
        set((state) => ({
          aiProviders: state.aiProviders.map((p) =>
            p.id === id ? { ...p, ...updates } : p
          ),
        }));
      },

      setBlockAds: (value: boolean) => set({ blockAds: value }),
      setBlockTrackers: (value: boolean) => set({ blockTrackers: value }),
      setEnableJavaScript: (value: boolean) => set({ enableJavaScript: value }),
      setClearOnExit: (value: boolean) => set({ clearOnExit: value }),
    }),
    {
      name: 'settings-storage',
      storage: createJSONStorage(() => AsyncStorage),
    }
  )
);
