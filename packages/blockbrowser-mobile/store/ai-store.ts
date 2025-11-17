import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import AsyncStorage from '@react-native-async-storage/async-storage';

export interface AIMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: number;
  providerId?: string;
  modelId?: string;
  error?: string;
}

export interface ChatSession {
  id: string;
  title: string;
  messages: AIMessage[];
  createdAt: number;
  updatedAt: number;
  providerId: string;
}

interface AIState {
  sessions: ChatSession[];
  currentSessionId: string | null;
  isStreaming: boolean;
  streamingMessageId: string | null;

  // Session Actions
  createSession: (providerId: string) => string;
  switchSession: (sessionId: string) => void;
  deleteSession: (sessionId: string) => void;
  getCurrentSession: () => ChatSession | null;
  clearAllSessions: () => void;

  // Message Actions
  addMessage: (
    sessionId: string,
    role: AIMessage['role'],
    content: string,
    providerId?: string,
    modelId?: string
  ) => string;
  updateMessage: (sessionId: string, messageId: string, content: string) => void;
  deleteMessage: (sessionId: string, messageId: string) => void;
  setMessageError: (sessionId: string, messageId: string, error: string) => void;

  // Streaming Actions
  startStreaming: (messageId: string) => void;
  stopStreaming: () => void;
  appendToStreamingMessage: (sessionId: string, content: string) => void;
}

export const useAIStore = create<AIState>()(
  persist(
    (set, get) => ({
      sessions: [],
      currentSessionId: null,
      isStreaming: false,
      streamingMessageId: null,

      createSession: (providerId: string) => {
        const newSession: ChatSession = {
          id: `session_${Date.now()}_${Math.random().toString(36).substring(7)}`,
          title: 'New Chat',
          messages: [],
          createdAt: Date.now(),
          updatedAt: Date.now(),
          providerId,
        };

        set((state) => ({
          sessions: [newSession, ...state.sessions],
          currentSessionId: newSession.id,
        }));

        return newSession.id;
      },

      switchSession: (sessionId: string) => {
        set({ currentSessionId: sessionId });
      },

      deleteSession: (sessionId: string) => {
        const { sessions, currentSessionId } = get();
        const newSessions = sessions.filter((s) => s.id !== sessionId);

        // If deleting current session, switch to next one
        let newCurrentId = currentSessionId;
        if (sessionId === currentSessionId) {
          newCurrentId = newSessions.length > 0 ? newSessions[0].id : null;
        }

        set({
          sessions: newSessions,
          currentSessionId: newCurrentId,
        });
      },

      getCurrentSession: () => {
        const { sessions, currentSessionId } = get();
        return sessions.find((s) => s.id === currentSessionId) || null;
      },

      clearAllSessions: () => {
        set({ sessions: [], currentSessionId: null });
      },

      addMessage: (
        sessionId: string,
        role: AIMessage['role'],
        content: string,
        providerId?: string,
        modelId?: string
      ) => {
        const newMessage: AIMessage = {
          id: `msg_${Date.now()}_${Math.random().toString(36).substring(7)}`,
          role,
          content,
          timestamp: Date.now(),
          providerId,
          modelId,
        };

        set((state) => ({
          sessions: state.sessions.map((session) => {
            if (session.id === sessionId) {
              // Update session title based on first user message
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

      updateMessage: (sessionId: string, messageId: string, content: string) => {
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

      setMessageError: (sessionId: string, messageId: string, error: string) => {
        set((state) => ({
          sessions: state.sessions.map((session) => {
            if (session.id === sessionId) {
              return {
                ...session,
                messages: session.messages.map((msg) =>
                  msg.id === messageId ? { ...msg, error } : msg
                ),
              };
            }
            return session;
          }),
        }));
      },

      startStreaming: (messageId: string) => {
        set({ isStreaming: true, streamingMessageId: messageId });
      },

      stopStreaming: () => {
        set({ isStreaming: false, streamingMessageId: null });
      },

      appendToStreamingMessage: (sessionId: string, content: string) => {
        const { streamingMessageId } = get();
        if (!streamingMessageId) return;

        set((state) => ({
          sessions: state.sessions.map((session) => {
            if (session.id === sessionId) {
              return {
                ...session,
                messages: session.messages.map((msg) =>
                  msg.id === streamingMessageId
                    ? { ...msg, content: msg.content + content }
                    : msg
                ),
              };
            }
            return session;
          }),
        }));
      },
    }),
    {
      name: 'ai-storage',
      storage: createJSONStorage(() => AsyncStorage),
      partialize: (state) => ({
        sessions: state.sessions.slice(0, 50), // Keep last 50 sessions
        currentSessionId: state.currentSessionId,
      }),
    }
  )
);
