import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { z } from 'zod'
import { Agent } from '../stores/agentsStore'
import { Logging } from '@/lib/utils/Logging'
import { getNemoAdapter } from '@/lib/browser/NemoAdapter'

// Provider schema
export const ProviderSchema = z.object({
  id: z.string(),
  name: z.string(),
  category: z.enum(['llm', 'search']),
  actionType: z.enum(['url', 'sidepanel']),
  urlPattern: z.string().optional(),
  searchParam: z.string().optional(),
  available: z.boolean().default(true),
  isCustom: z.boolean().optional(),
  openIn: z.enum(['newTab', 'current']).optional(),
  autoSubmit: z.boolean().optional(),
  submitKey: z.string().optional(),
  focusBeforeSubmit: z.boolean().optional(),
  iconUrl: z.string().optional()
})

export type Provider = z.infer<typeof ProviderSchema>

const CHAT_PROVIDER_READY_TIMEOUT_MS = 8000
const CHAT_PROVIDER_POST_LOAD_DELAY_MS = 400

const DEFAULT_PROVIDERS: Provider[] = [
  {
    id: 'nemo-agent',
    name: 'Nemo Agent',
    category: 'llm',
    actionType: 'sidepanel',
    available: true,
    iconUrl: '/assets/new_tab_search/nemo.svg'
  },
  {
    id: 'chatgpt',
    name: 'ChatGPT',
    category: 'llm',
    actionType: 'url',
    urlPattern: 'https://chatgpt.com',
    available: true,
    openIn: 'newTab',
    autoSubmit: true,
    submitKey: 'Enter',
    focusBeforeSubmit: true,
    iconUrl: '/assets/new_tab_search/openai.svg'
  },
  {
    id: 'claude',
    name: 'Claude',
    category: 'llm',
    actionType: 'url',
    urlPattern: 'https://claude.ai/new',
    available: true,
    openIn: 'newTab',
    autoSubmit: true,
    submitKey: 'Enter',
    focusBeforeSubmit: true,
    iconUrl: '/assets/new_tab_search/claude.svg'
  },
  {
    id: 'google',
    name: 'Google',
    category: 'search',
    actionType: 'url',
    urlPattern: 'https://www.google.com',
    available: true,
    openIn: 'newTab',
    autoSubmit: true,
    submitKey: 'Enter',
    focusBeforeSubmit: true,
    iconUrl: '/assets/new_tab_search/google.svg'
  }
]

const LEGACY_LOCAL_STORAGE_KEY = 'searchProviders'

type ProviderOrderState = Pick<ProviderState, 'providers' | 'customProviders' | 'enabledProviderIds' | 'disabledProviderIds'>

function ensureProtocol(url: string) {
  const trimmed = url.trim()
  if (!trimmed) return ''
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(trimmed)) {
    return trimmed
  }
  return `https://${trimmed}`
}

function deriveFaviconUrl(urlPattern?: string) {
  if (!urlPattern) return undefined
  try {
    const normalized = ensureProtocol(urlPattern)
    const parsed = new URL(normalized)
    return `https://www.google.com/s2/favicons?domain=${parsed.hostname}&sz=64`
  } catch {
    return undefined
  }
}

function normalizeProviderOrders(state: ProviderOrderState) {
  const allProviders = [...state.providers, ...state.customProviders]
  const providerMap = new Map(allProviders.map(provider => [provider.id, provider]))

  const uniqueEnabled: string[] = []
  state.enabledProviderIds.forEach(id => {
    if (providerMap.has(id) && !uniqueEnabled.includes(id)) {
      uniqueEnabled.push(id)
    }
  })

  const uniqueDisabled: string[] = []
  state.disabledProviderIds.forEach(id => {
    if (providerMap.has(id) && !uniqueEnabled.includes(id) && !uniqueDisabled.includes(id)) {
      uniqueDisabled.push(id)
    }
  })

  allProviders.forEach(provider => {
    if (!uniqueEnabled.includes(provider.id) && !uniqueDisabled.includes(provider.id)) {
      uniqueEnabled.push(provider.id)
    }
  })

  return {
    enabledProviderIds: uniqueEnabled,
    disabledProviderIds: uniqueDisabled
  }
}

function moveItem<T>(items: T[], from: number, to: number) {
  if (from === to) return items
  const list = [...items]
  const [item] = list.splice(from, 1)
  list.splice(to, 0, item)
  return list
}


function buildQueryInjectionScript(query: string) {
  return `
    (function() {
      const value = ${JSON.stringify(query)};
      const selectorGroups = [
        'input[name="search_query"]',
        'input[type="search"]', 
        'textarea[id="prompt-textarea"]',
        'textarea[data-id="chat-input"]',
        'textarea[data-testid="textbox"]',
        'input[type="text"]',
        'textarea',
        'div[contenteditable="true"]',
        'div[role="textbox"]',
        '[data-testid="textbox"]',
        '[role="textbox"]'
      ];

      const isVisible = (el) => {
        if (!el) return false;
        const style = window.getComputedStyle(el);
        if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') {
          return false;
        }
        const rect = el.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0;
      };

      const fireEvents = (el) => {
        try {
          el.dispatchEvent(new InputEvent('beforeinput', { bubbles: true, inputType: 'insertFromPaste', data: value }));
        } catch (_) {
          el.dispatchEvent(new Event('beforeinput', { bubbles: true }));
        }
        try {
          el.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertFromPaste', data: value }));
        } catch (_) {
          el.dispatchEvent(new Event('input', { bubbles: true }));
        }
        el.dispatchEvent(new Event('change', { bubbles: true }));
      };

      const setValue = (target) => {
        if (!target) return false;
        if (target.tagName === 'TEXTAREA' || target.tagName === 'INPUT') {
          const descriptor = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(target), 'value');
          if (descriptor && descriptor.set) {
            descriptor.set.call(target, value);
          } else {
            target.value = value;
          }
          target.focus();
          fireEvents(target);
          return true;
        }
        if (target.isContentEditable || target.getAttribute('contenteditable') === 'true') {
          target.focus();
          let success = false;
          if (typeof document.execCommand === 'function') {
            try {
              success = document.execCommand('insertText', false, value);
            } catch (_) {
              success = false;
            }
          }
          if (!success) {
            target.innerHTML = '';
            const paragraph = document.createElement('p');
            paragraph.textContent = value;
            target.appendChild(paragraph);
          }
          fireEvents(target);
          return true;
        }
        return false;
      };

      const seen = new Set();
      const searchRoot = (root) => {
        if (!root) return null;
        for (const selector of selectorGroups) {
          const el = root.querySelector?.(selector);
          if (el && !seen.has(el) && isVisible(el)) {
            return el;
          }
        }
        const all = root.querySelectorAll?.('*') || [];
        for (const el of all) {
          if (seen.has(el)) continue;
          seen.add(el);
          if (!isVisible(el)) continue;
          if (el.shadowRoot) {
            const deep = searchRoot(el.shadowRoot);
            if (deep) return deep;
          }
        }
        return null;
      };

      const target = searchRoot(document) || (document.activeElement && isVisible(document.activeElement) ? document.activeElement : null);
      if (!target) {
        return false;
      }
      return setValue(target);
    })();
  `;
}

interface ProviderState {
  providers: Provider[]
  customProviders: Provider[]
  selectedProviderId: string
  isDropdownOpen: boolean
  enabledProviderIds: string[]
  disabledProviderIds: string[]
  hasLegacySynced: boolean
}

interface ProviderActions {
  selectProvider: (id: string) => void
  toggleDropdown: () => void
  closeDropdown: () => void
  getSelectedProvider: () => Provider | undefined
  getProvidersByCategory: (category: 'llm' | 'search') => Provider[]
  addCustomProvider: (provider: Omit<Provider, 'id' | 'isCustom' | 'available'>) => void
  removeCustomProvider: (id: string) => void
  getEnabledProviders: () => Provider[]
  getDisabledProviders: () => Provider[]
  enableProvider: (id: string, position?: number) => void
  disableProvider: (id: string, position?: number) => void
  reorderEnabledProviders: (fromIndex: number, toIndex: number) => void
  reorderDisabledProviders: (fromIndex: number, toIndex: number) => void
  importLegacyProviderSettings: () => void
  executeProviderAction: (provider: Provider, query: string) => Promise<void>
  executeAgent: (agent: Agent, query: string, isBuilder?: boolean) => Promise<void>
}

export const useProviderStore = create<ProviderState & ProviderActions>()(
  persist(
    (set, get) => ({
      providers: DEFAULT_PROVIDERS,
      customProviders: [],
      selectedProviderId: 'nemo-agent',
      isDropdownOpen: false,
      enabledProviderIds: DEFAULT_PROVIDERS.map(provider => provider.id),
      disabledProviderIds: [],
      hasLegacySynced: false,

      selectProvider: id => {
        const { providers, customProviders } = get()
        const exists = [...providers, ...customProviders].some(provider => provider.id === id)
        if (!exists) return
        set({ selectedProviderId: id, isDropdownOpen: false })
      },

      toggleDropdown: () => set(state => ({ isDropdownOpen: !state.isDropdownOpen })),

      closeDropdown: () => set({ isDropdownOpen: false }),

      getSelectedProvider: () => {
        const state = get()
        const all = [...state.providers, ...state.customProviders]
        const selected = all.find(provider => provider.id === state.selectedProviderId)
        if (selected) return selected
        const [firstEnabled] = state.enabledProviderIds
          .map(id => all.find(provider => provider.id === id))
          .filter((provider): provider is Provider => Boolean(provider))
        return firstEnabled
      },

      getProvidersByCategory: category => {
        const state = get()
        const all = [...state.providers, ...state.customProviders]
        return all.filter(provider => provider.category === category)
      },

      addCustomProvider: provider => {
        const normalizedUrl = provider.urlPattern ? ensureProtocol(provider.urlPattern) : ''
        if (!normalizedUrl) return

        const id = crypto.randomUUID()
        const newProvider: Provider = {
          ...provider,
          id,
          urlPattern: normalizedUrl,
          isCustom: true,
          available: true,
          autoSubmit: provider.autoSubmit ?? true,
          focusBeforeSubmit: provider.focusBeforeSubmit ?? true,
          iconUrl: provider.iconUrl || deriveFaviconUrl(normalizedUrl)
        }

        set(state => {
          const nextState: ProviderOrderState & Partial<ProviderState> = {
            ...state,
            customProviders: [...state.customProviders, newProvider],
            enabledProviderIds: [...state.enabledProviderIds, newProvider.id],
            disabledProviderIds: state.disabledProviderIds
          }
          const normalized = normalizeProviderOrders(nextState)
          return {
            ...state,
            customProviders: nextState.customProviders,
            enabledProviderIds: normalized.enabledProviderIds,
            disabledProviderIds: normalized.disabledProviderIds
          }
        })
      },

      removeCustomProvider: id => {
        set(state => {
          const customProviders = state.customProviders.filter(provider => provider.id !== id)
          const isSelected = state.selectedProviderId === id
          const nextState: ProviderOrderState & Partial<ProviderState> = {
            ...state,
            customProviders,
            enabledProviderIds: state.enabledProviderIds.filter(providerId => providerId !== id),
            disabledProviderIds: state.disabledProviderIds.filter(providerId => providerId !== id)
          }
          const normalized = normalizeProviderOrders(nextState)
          const nextSelected = isSelected
            ? normalized.enabledProviderIds[0] || 'nemo-agent'
            : state.selectedProviderId

          return {
            ...state,
            customProviders,
            enabledProviderIds: normalized.enabledProviderIds,
            disabledProviderIds: normalized.disabledProviderIds,
            selectedProviderId: nextSelected
          }
        })
      },

      getEnabledProviders: () => {
        const state = get()
        const all = [...state.providers, ...state.customProviders]
        const map = new Map(all.map(provider => [provider.id, provider]))
        return state.enabledProviderIds
          .map(id => map.get(id))
          .filter((provider): provider is Provider => Boolean(provider))
      },

      getDisabledProviders: () => {
        const state = get()
        const all = [...state.providers, ...state.customProviders]
        const map = new Map(all.map(provider => [provider.id, provider]))
        return state.disabledProviderIds
          .map(id => map.get(id))
          .filter((provider): provider is Provider => Boolean(provider))
      },

      enableProvider: (id, position) => {
        set(state => {
          if (state.enabledProviderIds.includes(id)) return state
          const enabled = [...state.enabledProviderIds]
          if (position === undefined) {
            enabled.push(id)
          } else {
            enabled.splice(position, 0, id)
          }
          const nextState: ProviderOrderState & Partial<ProviderState> = {
            ...state,
            enabledProviderIds: enabled,
            disabledProviderIds: state.disabledProviderIds.filter(providerId => providerId !== id)
          }
          const normalized = normalizeProviderOrders(nextState)
          return {
            ...state,
            enabledProviderIds: normalized.enabledProviderIds,
            disabledProviderIds: normalized.disabledProviderIds
          }
        })
      },

      disableProvider: (id, position) => {
        set(state => {
          const enabled = state.enabledProviderIds.filter(providerId => providerId !== id)
          const disabled = [...state.disabledProviderIds.filter(providerId => providerId !== id)]
          if (position === undefined) {
            disabled.push(id)
          } else {
            disabled.splice(position, 0, id)
          }
          const nextState: ProviderOrderState & Partial<ProviderState> = {
            ...state,
            enabledProviderIds: enabled,
            disabledProviderIds: disabled
          }
          const normalized = normalizeProviderOrders(nextState)
          const selectedProviderId = state.selectedProviderId === id
            ? normalized.enabledProviderIds[0] || 'nemo-agent'
            : state.selectedProviderId
          return {
            ...state,
            enabledProviderIds: normalized.enabledProviderIds,
            disabledProviderIds: normalized.disabledProviderIds,
            selectedProviderId
          }
        })
      },

      reorderEnabledProviders: (fromIndex, toIndex) => {
        set(state => {
          if (fromIndex < 0 || toIndex < 0) return state
          if (fromIndex >= state.enabledProviderIds.length || toIndex >= state.enabledProviderIds.length) {
            return state
          }
          const enabled = moveItem(state.enabledProviderIds, fromIndex, toIndex)
          const nextState: ProviderOrderState & Partial<ProviderState> = {
            ...state,
            enabledProviderIds: enabled
          }
          const normalized = normalizeProviderOrders(nextState)
          return {
            ...state,
            enabledProviderIds: normalized.enabledProviderIds,
            disabledProviderIds: normalized.disabledProviderIds
          }
        })
      },

      reorderDisabledProviders: (fromIndex, toIndex) => {
        set(state => {
          if (fromIndex < 0 || toIndex < 0) return state
          if (fromIndex >= state.disabledProviderIds.length || toIndex >= state.disabledProviderIds.length) {
            return state
          }
          const disabled = moveItem(state.disabledProviderIds, fromIndex, toIndex)
          const nextState: ProviderOrderState & Partial<ProviderState> = {
            ...state,
            disabledProviderIds: disabled
          }
          const normalized = normalizeProviderOrders(nextState)
          return {
            ...state,
            enabledProviderIds: normalized.enabledProviderIds,
            disabledProviderIds: normalized.disabledProviderIds
          }
        })
      },

      importLegacyProviderSettings: () => {
        const state = get()
        if (state.hasLegacySynced) return
        if (typeof window === 'undefined') return

        try {
          const raw = window.localStorage.getItem(LEGACY_LOCAL_STORAGE_KEY)
          if (!raw) {
            set({ hasLegacySynced: true })
            return
          }

          const parsed = JSON.parse(raw)
          const enabledLegacy: string[] = Array.isArray(parsed?.enabled)
            ? parsed.enabled.map((provider: { id: string }) => provider?.id).filter((id: string) => typeof id === 'string')
            : []
          const disabledLegacy: string[] = Array.isArray(parsed?.disabled)
            ? parsed.disabled.map((provider: { id: string }) => provider?.id).filter((id: string) => typeof id === 'string')
            : []

          const mapLegacyId = (id: string) => (id === 'nemo' ? 'nemo-agent' : id)

          const enabledProviderIds = enabledLegacy.map(mapLegacyId)
          const disabledProviderIds = disabledLegacy.map(mapLegacyId)

          set(current => {
            const nextState: ProviderOrderState & Partial<ProviderState> = {
              ...current,
              enabledProviderIds,
              disabledProviderIds
            }
            const normalized = normalizeProviderOrders(nextState)
            const nextSelected = normalized.enabledProviderIds.includes(current.selectedProviderId)
              ? current.selectedProviderId
              : normalized.enabledProviderIds[0] || 'nemo-agent'

            return {
              ...current,
              enabledProviderIds: normalized.enabledProviderIds,
              disabledProviderIds: normalized.disabledProviderIds,
              hasLegacySynced: true,
              selectedProviderId: nextSelected
            }
          })

          window.localStorage.removeItem(LEGACY_LOCAL_STORAGE_KEY)
        } catch (error) {
          console.error('Failed to import legacy provider settings', error)
          set({ hasLegacySynced: true })
        }
      },

      executeProviderAction: async (provider, query) => {
        Logging.logMetric('newtab.execute_provider', {
          providerName: provider.name,
          actionType: provider.actionType
        })

        if (provider.actionType === 'url' && provider.urlPattern) {
          const hasPlaceholder = provider.urlPattern.includes('%s')
          const targetUrl = hasPlaceholder
            ? provider.urlPattern.replace('%s', encodeURIComponent(query))
            : provider.urlPattern

          const openInNewTab = provider.openIn === 'newTab' ||
            (provider.openIn === undefined && provider.category === 'llm')

          let tabId: number | undefined
          let nemo = getNemoAdapter()

          try {
            if (openInNewTab) {
              const tab = await chrome.tabs.create({ url: targetUrl })
              tabId = tab.id
            } else {
              const [activeTab] = await chrome.tabs.query({ active: true, currentWindow: true })
              if (activeTab?.id) {
                await chrome.tabs.update(activeTab.id, { url: targetUrl })
                tabId = activeTab.id
              }
            }

            const needsDomWork = Boolean(tabId != null && (!hasPlaceholder || provider.autoSubmit))
            let queryInjected = false

            if (needsDomWork && tabId != null) {
              const start = Date.now()
              while (Date.now() - start < CHAT_PROVIDER_READY_TIMEOUT_MS) {
                try {
                  const status = await nemo.getPageLoadStatus(tabId)
                  if (status.isDOMContentLoaded) {
                    break
                  }
                } catch (_) {
                  // Ignore transient errors while polling
                }
                await new Promise(resolve => setTimeout(resolve, 100))
              }

              await new Promise(resolve => setTimeout(resolve, CHAT_PROVIDER_POST_LOAD_DELAY_MS))

              if (!hasPlaceholder) {
                const injectQuery = async () => {
                  if (tabId == null) return false;
                  try {
                    const script = buildQueryInjectionScript(query);
                    const result = await nemo.executeJavaScript(tabId, script);
                    return Boolean(result);
                  } catch (error) {
                    console.warn('Failed to inject query for provider', provider.id, error);
                    return false;
                  }
                };

                queryInjected = await injectQuery();

                if (!queryInjected) {
                  for (let attempt = 0; attempt < 5 && !queryInjected; attempt++) {
                    await new Promise(resolve => setTimeout(resolve, 400));
                    queryInjected = await injectQuery();
                  }
                }
              }

              if (provider.focusBeforeSubmit) {
                try {
                  await nemo.executeJavaScript(tabId, `
                    (function() {
                      const el = document.querySelector('textarea, [contenteditable="true"], input[type="search"], input[type="text"]');
                      if (el) el.focus();
                    })();
                  `)
                } catch (error) {
                  console.warn('Failed to focus input for provider', provider.id, error)
                }
              }
            }

            if (provider.autoSubmit && tabId != null) {
              if (hasPlaceholder || queryInjected) {
                const submitKey = provider.submitKey || 'Enter'
                await nemo.sendKeys(tabId, submitKey as chrome.nemo.Key)
              }
            }
          } catch (error) {
            console.error(`Failed to execute provider ${provider.id}:`, error)
            if (openInNewTab) {
              window.open(targetUrl, '_blank')
            }
          }
        } else if (provider.actionType === 'sidepanel') {
          try {
            const [activeTab] = await chrome.tabs.query({ active: true, currentWindow: true })
            if (!activeTab?.id) {
              console.error('No active tab found')
              return
            }
            await chrome.runtime.sendMessage({
              type: 'NEWTAB_EXECUTE_QUERY',
              tabId: activeTab.id,
              query,
              metadata: {
                source: 'newtab',
                executionMode: 'dynamic'
              }
            })
          } catch (error) {
            console.error('Failed to execute query from newtab:', error)
          }
        } else {
          console.warn(`No action defined for provider: ${provider.id}`)
        }
      },

      executeAgent: async (agent, query, isBuilder) => {
        Logging.logMetric('newtab.execute_agent', {
          agentName: agent.name
        })

        try {
          const [activeTab] = await chrome.tabs.query({ active: true, currentWindow: true })
          if (!activeTab?.id) {
            console.error('No active tab found')
            return
          }

          await chrome.runtime.sendMessage({
            type: 'NEWTAB_EXECUTE_QUERY',
            tabId: activeTab.id,
            query,
            metadata: {
              source: 'newtab'
            }
          })
        } catch (error) {
          console.error('Failed to execute agent:', error)
        }
      }
    }),
    {
      name: 'nemo-providers',
      version: 2,
      migrate: (persistedState, version) => {
        if (!persistedState) return persistedState
        if (version >= 2) return persistedState

        const legacyState = persistedState as Partial<ProviderState>
        const customProviders = legacyState.customProviders || []

        const enabledProviderIds = (legacyState as any)?.enabledProviderIds || DEFAULT_PROVIDERS.map(provider => provider.id)
        const disabledProviderIds = (legacyState as any)?.disabledProviderIds || []

        const baseState: ProviderOrderState & Partial<ProviderState> = {
          providers: DEFAULT_PROVIDERS,
          customProviders,
          enabledProviderIds,
          disabledProviderIds
        }

        const normalized = normalizeProviderOrders(baseState)

        return {
          providers: DEFAULT_PROVIDERS,
          customProviders,
          selectedProviderId: legacyState.selectedProviderId || 'nemo-agent',
          isDropdownOpen: false,
          enabledProviderIds: normalized.enabledProviderIds,
          disabledProviderIds: normalized.disabledProviderIds,
          hasLegacySynced: false
        }
      }
    }
  )
)

