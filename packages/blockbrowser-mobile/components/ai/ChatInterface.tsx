import React, { useRef, useEffect, useState } from 'react';
import {
  View,
  FlatList,
  TextInput,
  TouchableOpacity,
  StyleSheet,
  KeyboardAvoidingView,
  Platform,
  useColorScheme,
  ActivityIndicator,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { LiquidGlassView } from '@callstack/liquid-glass';
import { isLiquidGlassSupported } from '@/lib/utils/liquid-glass-helper';
import { Colors } from '@/constants/Colors';
import { useAIStore } from '@/store/ai-store';
import { AIProviderFactory } from '@/lib/ai/providers/factory';
import { SecureStorage } from '@/lib/storage/secure-store';
import { MessageBubble } from './MessageBubble';

interface ChatInterfaceProps {
  sessionId: string;
}

export const ChatInterface: React.FC<ChatInterfaceProps> = ({ sessionId }) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;
  const useLiquidGlass = Platform.OS === 'ios' && isLiquidGlassSupported();

  const flatListRef = useRef<FlatList>(null);
  const [inputText, setInputText] = useState('');
  const [isSending, setIsSending] = useState(false);

  const { sessions, addMessage, updateMessageContent, startStreaming, endStreaming } = useAIStore();
  const session = sessions.find((s) => s.id === sessionId);

  const messages = session?.messages || [];
  const streamingMessageId = messages.find((m) => m.isStreaming)?.id;

  useEffect(() => {
    // Scroll to bottom when new messages arrive
    if (messages.length > 0) {
      setTimeout(() => {
        flatListRef.current?.scrollToEnd({ animated: true });
      }, 100);
    }
  }, [messages.length]);

  const handleSend = async () => {
    if (!inputText.trim() || !session || isSending) return;

    const userMessage = inputText.trim();
    setInputText('');
    setIsSending(true);

    try {
      // Add user message
      const userMessageId = addMessage(sessionId, 'user', userMessage);

      // Add AI message placeholder
      const aiMessageId = addMessage(sessionId, 'assistant', '');
      startStreaming(aiMessageId);

      // Get provider configuration
      const provider = AIProviderFactory.getProvider(session.providerType as any);
      const apiKey = await SecureStorage.getApiKey(session.providerId);

      if (!apiKey) {
        updateMessageContent(
          sessionId,
          aiMessageId,
          'Error: API key not found. Please configure it in Settings.'
        );
        endStreaming(aiMessageId);
        setIsSending(false);
        return;
      }

      // Get conversation history
      const conversationMessages = messages
        .concat([
          { id: userMessageId, role: 'user', content: userMessage, timestamp: Date.now() },
        ])
        .map((m) => ({
          role: m.role,
          content: m.content,
        }));

      // Stream AI response
      let fullMessage = '';
      await provider.chat(
        conversationMessages,
        session.model,
        apiKey,
        {
          onStream: (chunk) => {
            fullMessage += chunk.content;
            updateMessageContent(sessionId, aiMessageId, fullMessage);
          },
        }
      );

      endStreaming(aiMessageId);
    } catch (error) {
      console.error('blockbrowser: Error sending message:', error);
      const errorMessage = error instanceof Error ? error.message : 'Unknown error occurred';

      // Find the last AI message and update it with error
      const lastAIMessage = messages
        .filter((m) => m.role === 'assistant')
        .pop();

      if (lastAIMessage) {
        updateMessageContent(
          sessionId,
          lastAIMessage.id,
          `Error: ${errorMessage}`
        );
        endStreaming(lastAIMessage.id);
      }
    } finally {
      setIsSending(false);
    }
  };

  const InputContainer = () => (
    <View
      style={[
        styles.inputContainer,
        {
          backgroundColor: colors.surface,
          borderTopColor: colors.border,
        },
      ]}
    >
      <TextInput
        style={[
          styles.input,
          {
            color: colors.text,
            backgroundColor: colors.background,
          },
        ]}
        value={inputText}
        onChangeText={setInputText}
        placeholder="Ask anything..."
        placeholderTextColor={colors.textSecondary}
        multiline
        maxLength={4000}
        editable={!isSending}
      />
      <TouchableOpacity
        style={[
          styles.sendButton,
          {
            backgroundColor: inputText.trim() ? colors.primary : colors.border,
          },
        ]}
        onPress={handleSend}
        disabled={!inputText.trim() || isSending}
      >
        {isSending ? (
          <ActivityIndicator size="small" color={colors.background} />
        ) : (
          <Ionicons
            name="send"
            size={20}
            color={inputText.trim() ? colors.background : colors.textSecondary}
          />
        )}
      </TouchableOpacity>
    </View>
  );

  return (
    <KeyboardAvoidingView
      style={styles.container}
      behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
      keyboardVerticalOffset={Platform.OS === 'ios' ? 90 : 0}
    >
      {/* Messages List */}
      <FlatList
        ref={flatListRef}
        data={messages}
        keyExtractor={(item) => item.id}
        renderItem={({ item }) => (
          <MessageBubble
            message={item}
            isStreaming={item.id === streamingMessageId}
          />
        )}
        contentContainerStyle={styles.messagesList}
        showsVerticalScrollIndicator={false}
      />

      {/* Input Area */}
      {useLiquidGlass ? (
        <LiquidGlassView
          style={styles.glassInput}
          effect="prominent"
          tintColor={colorScheme === 'dark' ? '#1c1c1e' : '#f2f2f7'}
          colorScheme={colorScheme || 'light'}
        >
          <InputContainer />
        </LiquidGlassView>
      ) : (
        <InputContainer />
      )}
    </KeyboardAvoidingView>
  );
};

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  messagesList: {
    paddingTop: 16,
    paddingBottom: 8,
  },
  glassInput: {
    borderTopWidth: 1,
    borderTopColor: 'rgba(0,0,0,0.1)',
  },
  inputContainer: {
    flexDirection: 'row',
    alignItems: 'flex-end',
    padding: 12,
    borderTopWidth: 1,
    gap: 8,
  },
  input: {
    flex: 1,
    borderRadius: 20,
    paddingHorizontal: 16,
    paddingVertical: 10,
    fontSize: 15,
    maxHeight: 100,
  },
  sendButton: {
    width: 40,
    height: 40,
    borderRadius: 20,
    alignItems: 'center',
    justifyContent: 'center',
  },
});
