import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import AsyncStorage from '@react-native-async-storage/async-storage';

export interface Tab {
  id: string;
  url: string;
  title: string;
  favicon?: string;
  canGoBack: boolean;
  canGoForward: boolean;
  isLoading: boolean;
  progress: number;
  createdAt: number;
  lastAccessed: number;
}

export interface BrowserHistory {
  id: string;
  url: string;
  title: string;
  timestamp: number;
}

export interface Bookmark {
  id: string;
  url: string;
  title: string;
  folder?: string;
  createdAt: number;
}

interface BrowserState {
  tabs: Tab[];
  activeTabId: string | null;
  history: BrowserHistory[];
  bookmarks: Bookmark[];
  incognitoMode: boolean;

  // Tab Actions
  addTab: (url: string) => string;
  closeTab: (id: string) => void;
  switchTab: (id: string) => void;
  updateTab: (id: string, updates: Partial<Tab>) => void;
  getActiveTab: () => Tab | null;
  clearAllTabs: () => void;

  // History Actions
  addToHistory: (url: string, title: string) => void;
  clearHistory: () => void;
  searchHistory: (query: string) => BrowserHistory[];

  // Bookmark Actions
  addBookmark: (url: string, title: string, folder?: string) => void;
  removeBookmark: (id: string) => void;
  getBookmarkByUrl: (url: string) => Bookmark | null;

  // Settings
  toggleIncognitoMode: () => void;
}

export const useBrowserStore = create<BrowserState>()(
  persist(
    (set, get) => ({
      tabs: [],
      activeTabId: null,
      history: [],
      bookmarks: [],
      incognitoMode: false,

      addTab: (url: string) => {
        const newTab: Tab = {
          id: `tab_${Date.now()}_${Math.random().toString(36).substring(7)}`,
          url,
          title: 'New Tab',
          canGoBack: false,
          canGoForward: false,
          isLoading: true,
          progress: 0,
          createdAt: Date.now(),
          lastAccessed: Date.now(),
        };

        set((state) => ({
          tabs: [...state.tabs, newTab],
          activeTabId: newTab.id,
        }));

        return newTab.id;
      },

      closeTab: (id: string) => {
        const { tabs, activeTabId } = get();
        const newTabs = tabs.filter((tab) => tab.id !== id);

        // If no tabs left, don't update
        if (newTabs.length === 0) {
          set({ tabs: [], activeTabId: null });
          return;
        }

        // If closing active tab, switch to next tab
        let newActiveId = activeTabId;
        if (id === activeTabId) {
          const closedIndex = tabs.findIndex((t) => t.id === id);
          const nextIndex = Math.min(closedIndex, newTabs.length - 1);
          newActiveId = newTabs[nextIndex]?.id || null;
        }

        set({ tabs: newTabs, activeTabId: newActiveId });
      },

      switchTab: (id: string) => {
        set((state) => ({
          activeTabId: id,
          tabs: state.tabs.map((tab) =>
            tab.id === id ? { ...tab, lastAccessed: Date.now() } : tab
          ),
        }));
      },

      updateTab: (id: string, updates: Partial<Tab>) => {
        set((state) => ({
          tabs: state.tabs.map((tab) =>
            tab.id === id ? { ...tab, ...updates } : tab
          ),
        }));
      },

      getActiveTab: () => {
        const { tabs, activeTabId } = get();
        return tabs.find((tab) => tab.id === activeTabId) || null;
      },

      clearAllTabs: () => {
        set({ tabs: [], activeTabId: null });
      },

      addToHistory: (url: string, title: string) => {
        const { incognitoMode, history } = get();

        // Don't save to history in incognito mode
        if (incognitoMode) return;

        const newEntry: BrowserHistory = {
          id: `history_${Date.now()}`,
          url,
          title,
          timestamp: Date.now(),
        };

        // Remove duplicate URLs and add new entry
        const filteredHistory = history.filter((h) => h.url !== url);
        const newHistory = [newEntry, ...filteredHistory].slice(0, 1000); // Keep last 1000

        set({ history: newHistory });
      },

      clearHistory: () => set({ history: [] }),

      searchHistory: (query: string) => {
        const { history } = get();
        const lowerQuery = query.toLowerCase();

        return history.filter(
          (h) =>
            h.title.toLowerCase().includes(lowerQuery) ||
            h.url.toLowerCase().includes(lowerQuery)
        );
      },

      addBookmark: (url: string, title: string, folder?: string) => {
        const newBookmark: Bookmark = {
          id: `bookmark_${Date.now()}`,
          url,
          title,
          folder,
          createdAt: Date.now(),
        };

        set((state) => ({
          bookmarks: [...state.bookmarks, newBookmark],
        }));
      },

      removeBookmark: (id: string) => {
        set((state) => ({
          bookmarks: state.bookmarks.filter((b) => b.id !== id),
        }));
      },

      getBookmarkByUrl: (url: string) => {
        const { bookmarks } = get();
        return bookmarks.find((b) => b.url === url) || null;
      },

      toggleIncognitoMode: () => {
        set((state) => ({ incognitoMode: !state.incognitoMode }));
      },
    }),
    {
      name: 'browser-storage',
      storage: createJSONStorage(() => AsyncStorage),
      partialize: (state) => ({
        tabs: state.tabs,
        history: state.history,
        bookmarks: state.bookmarks,
        // Don't persist incognito mode
      }),
    }
  )
);
