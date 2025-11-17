import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { AIProvider, ProviderType, PROVIDER_DEFAULTS } from '@/constants/Providers';
import { Theme } from '@/constants/Colors';

interface SettingsState {
  theme: Theme;
  providers: AIProvider[];
  defaultProviderId: string;

  // Display Settings
  fontSize: number;
  showTabBar: boolean;
  enableHaptics: boolean;

  // Privacy Settings
  clearHistoryOnExit: boolean;
  blockTrackers: boolean;
  enableDNT: boolean;

  // Theme Actions
  setTheme: (theme: Theme) => void;

  // Provider Actions
  addProvider: (provider: Omit<AIProvider, 'id' | 'createdAt' | 'updatedAt'>) => string;
  updateProvider: (id: string, updates: Partial<AIProvider>) => void;
  removeProvider: (id: string) => void;
  setDefaultProvider: (id: string) => void;
  getProviderById: (id: string) => AIProvider | null;
  getDefaultProvider: () => AIProvider | null;

  // Settings Actions
  setFontSize: (size: number) => void;
  toggleTabBar: () => void;
  toggleHaptics: () => void;
  toggleClearHistory: () => void;
  toggleBlockTrackers: () => void;
  toggleDNT: () => void;

  // Reset
  resetToDefaults: () => void;
}

const createDefaultProvider = (): AIProvider => ({
  id: 'blockbrowser',
  name: 'BlockBrowser',
  type: ProviderType.BLOCKBROWSER,
  isDefault: true,
  isBuiltIn: true,
  capabilities: {
    supportsImages: true,
    supportsStreaming: true,
  },
  modelConfig: {
    contextWindow: 128000,
    temperature: 0.7,
  },
  createdAt: Date.now(),
  updatedAt: Date.now(),
});

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set, get) => ({
      theme: 'system',
      providers: [createDefaultProvider()],
      defaultProviderId: 'blockbrowser',
      fontSize: 16,
      showTabBar: true,
      enableHaptics: true,
      clearHistoryOnExit: false,
      blockTrackers: true,
      enableDNT: true,

      setTheme: (theme: Theme) => set({ theme }),

      addProvider: (providerData) => {
        const newProvider: AIProvider = {
          ...providerData,
          id: `provider_${Date.now()}_${Math.random().toString(36).substring(7)}`,
          createdAt: Date.now(),
          updatedAt: Date.now(),
        };

        set((state) => ({
          providers: [...state.providers, newProvider],
        }));

        return newProvider.id;
      },

      updateProvider: (id: string, updates: Partial<AIProvider>) => {
        set((state) => ({
          providers: state.providers.map((p) =>
            p.id === id
              ? { ...p, ...updates, updatedAt: Date.now() }
              : p
          ),
        }));
      },

      removeProvider: (id: string) => {
        const { defaultProviderId, providers } = get();

        // Don't remove built-in provider
        const provider = providers.find((p) => p.id === id);
        if (provider?.isBuiltIn) return;

        const newProviders = providers.filter((p) => p.id !== id);

        // If removing default provider, set built-in as default
        const newDefaultId =
          defaultProviderId === id ? 'blockbrowser' : defaultProviderId;

        set({
          providers: newProviders,
          defaultProviderId: newDefaultId,
        });
      },

      setDefaultProvider: (id: string) => {
        set({ defaultProviderId: id });
      },

      getProviderById: (id: string) => {
        const { providers } = get();
        return providers.find((p) => p.id === id) || null;
      },

      getDefaultProvider: () => {
        const { providers, defaultProviderId } = get();
        return providers.find((p) => p.id === defaultProviderId) || providers[0] || null;
      },

      setFontSize: (size: number) => set({ fontSize: size }),
      toggleTabBar: () => set((state) => ({ showTabBar: !state.showTabBar })),
      toggleHaptics: () => set((state) => ({ enableHaptics: !state.enableHaptics })),
      toggleClearHistory: () =>
        set((state) => ({ clearHistoryOnExit: !state.clearHistoryOnExit })),
      toggleBlockTrackers: () =>
        set((state) => ({ blockTrackers: !state.blockTrackers })),
      toggleDNT: () => set((state) => ({ enableDNT: !state.enableDNT })),

      resetToDefaults: () => {
        set({
          theme: 'system',
          providers: [createDefaultProvider()],
          defaultProviderId: 'blockbrowser',
          fontSize: 16,
          showTabBar: true,
          enableHaptics: true,
          clearHistoryOnExit: false,
          blockTrackers: true,
          enableDNT: true,
        });
      },
    }),
    {
      name: 'settings-storage',
      storage: createJSONStorage(() => AsyncStorage),
    }
  )
);
