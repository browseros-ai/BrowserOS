import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import AsyncStorage from '@react-native-async-storage/async-storage';

export interface AIMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: number;
  isStreaming?: boolean;
}

export interface ChatSession {
  id: string;
  title: string;
  messages: AIMessage[];
  providerId: string;
  providerType: string;
  model: string;
  createdAt: number;
  updatedAt: number;
}

interface AIState {
  sessions: ChatSession[];
  activeSessionId: string | null;

  // Session Actions
  createSession: (providerId: string, providerType: string, model: string) => string;
  setActiveSession: (sessionId: string) => void;
  deleteSession: (sessionId: string) => void;
  clearAllSessions: () => void;

  // Message Actions
  addMessage: (
    sessionId: string,
    role: AIMessage['role'],
    content: string
  ) => string;
  updateMessageContent: (sessionId: string, messageId: string, content: string) => void;
  deleteMessage: (sessionId: string, messageId: string) => void;

  // Streaming Actions
  startStreaming: (messageId: string) => void;
  endStreaming: (messageId: string) => void;
}

export const useAIStore = create<AIState>()(
  persist(
    (set, get) => ({
      sessions: [],
      activeSessionId: null,

      createSession: (providerId: string, providerType: string, model: string) => {
        const newSession: ChatSession = {
          id: `session_${ Date.now()}_${Math.random().toString(36).substring(7)}`,
          title: 'New Chat',
          messages: [],
          providerId,
          providerType,
          model,
          createdAt: Date.now(),
          updatedAt: Date.now(),
        };

        set((state) => ({
          sessions: [newSession, ...state.sessions],
          activeSessionId: newSession.id,
        }));

        return newSession.id;
      },

      setActiveSession: (sessionId: string) => {
        set({ activeSessionId: sessionId });
      },

      deleteSession: (sessionId: string) => {
        const { sessions, activeSessionId } = get();
        const newSessions = sessions.filter((s) => s.id !== sessionId);

        let newActiveId = activeSessionId;
        if (sessionId === activeSessionId) {
          newActiveId = newSessions.length > 0 ? newSessions[0].id : null;
        }

        set({
          sessions: newSessions,
          activeSessionId: newActiveId,
        });
      },

      clearAllSessions: () => {
        set({ sessions: [], activeSessionId: null });
      },

      addMessage: (
        sessionId: string,
        role: AIMessage['role'],
        content: string
      ) => {
        const newMessage: AIMessage = {
          id: `msg_${Date.now()}_${Math.random().toString(36).substring(7)}`,
          role,
          content,
          timestamp: Date.now(),
          isStreaming: false,
        };

        set((state) => ({
          sessions: state.sessions.map((session) => {
            if (session.id === sessionId) {
              let title = session.title;
              if (session.messages.length === 0 && role === 'user') {
                title = content.substring(0, 50) + (content.length > 50 ? '...' : '');
              }

              return {
                ...session,
                title,
                messages: [...session.messages, newMessage],
                updatedAt: Date.now(),
              };
            }
            return session;
          }),
        }));

        return newMessage.id;
      },

      updateMessageContent: (sessionId: string, messageId: string, content: string) => {
        set((state) => ({
          sessions: state.sessions.map((session) => {
            if (session.id === sessionId) {
              return {
                ...session,
                messages: session.messages.map((msg) =>
                  msg.id === messageId ? { ...msg, content } : msg
                ),
                updatedAt: Date.now(),
              };
            }
            return session;
          }),
        }));
      },

      deleteMessage: (sessionId: string, messageId: string) => {
        set((state) => ({
          sessions: state.sessions.map((session) => {
            if (session.id === sessionId) {
              return {
                ...session,
                messages: session.messages.filter((msg) => msg.id !== messageId),
                updatedAt: Date.now(),
              };
            }
            return session;
          }),
        }));
      },

      startStreaming: (messageId: string) => {
        set((state) => ({
          sessions: state.sessions.map((session) => ({
            ...session,
            messages: session.messages.map((msg) =>
              msg.id === messageId ? { ...msg, isStreaming: true } : msg
            ),
          })),
        }));
      },

      endStreaming: (messageId: string) => {
        set((state) => ({
          sessions: state.sessions.map((session) => ({
            ...session,
            messages: session.messages.map((msg) =>
              msg.id === messageId ? { ...msg, isStreaming: false } : msg
            ),
          })),
        }));
      },
    }),
    {
      name: 'ai-storage',
      storage: createJSONStorage(() => AsyncStorage),
      partialize: (state) => ({
        sessions: state.sessions.slice(0, 50),
        activeSessionId: state.activeSessionId,
      }),
    }
  )
);
