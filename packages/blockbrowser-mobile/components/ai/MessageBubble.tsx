import React from 'react';
import {
  View,
  Text,
  StyleSheet,
  useColorScheme,
  Platform,
} from 'react-native';
import { LiquidGlassView, isLiquidGlassSupported } from '@callstack/liquid-glass';
import { Ionicons } from '@expo/vector-icons';
import { Colors } from '@/constants/Colors';
import { AIMessage } from '@/store/ai-store';

interface MessageBubbleProps {
  message: AIMessage;
  isStreaming?: boolean;
}

export const MessageBubble: React.FC<MessageBubbleProps> = ({
  message,
  isStreaming = false,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;
  const useLiquidGlass = Platform.OS === 'ios' && isLiquidGlassSupported();

  const isUser = message.role === 'user';

  const BubbleContent = () => (
    <View
      style={[
        styles.bubble,
        isUser ? styles.userBubble : styles.aiBubble,
        {
          backgroundColor: isUser ? colors.primary : colors.surface,
        },
      ]}
    >
      {/* Role Icon */}
      <View style={styles.iconContainer}>
        {isUser ? (
          <Ionicons name="person-circle" size={24} color={colors.background} />
        ) : (
          <Ionicons name="sparkles" size={24} color={colors.primary} />
        )}
      </View>

      {/* Message Content */}
      <View style={styles.contentContainer}>
        <Text
          style={[
            styles.messageText,
            {
              color: isUser ? colors.background : colors.text,
            },
          ]}
        >
          {message.content}
        </Text>

        {/* Streaming Indicator */}
        {isStreaming && (
          <View style={styles.streamingIndicator}>
            <View style={[styles.dot, { backgroundColor: colors.primary }]} />
            <View style={[styles.dot, { backgroundColor: colors.primary }]} />
            <View style={[styles.dot, { backgroundColor: colors.primary }]} />
          </View>
        )}

        {/* Timestamp */}
        <Text
          style={[
            styles.timestamp,
            {
              color: isUser
                ? colors.background + 'CC'
                : colors.textSecondary,
            },
          ]}
        >
          {new Date(message.timestamp).toLocaleTimeString([], {
            hour: '2-digit',
            minute: '2-digit',
          })}
        </Text>
      </View>
    </View>
  );

  if (useLiquidGlass && !isUser) {
    return (
      <View style={[styles.messageContainer, styles.aiMessageContainer]}>
        <LiquidGlassView
          style={styles.glassBubble}
          effect="regular"
          interactive
          tintColor={colorScheme === 'dark' ? '#272728' : '#ffffff'}
          colorScheme={colorScheme || 'light'}
        >
          <BubbleContent />
        </LiquidGlassView>
      </View>
    );
  }

  return (
    <View
      style={[
        styles.messageContainer,
        isUser ? styles.userMessageContainer : styles.aiMessageContainer,
      ]}
    >
      <BubbleContent />
    </View>
  );
};

const styles = StyleSheet.create({
  messageContainer: {
    marginBottom: 16,
    paddingHorizontal: 16,
  },
  userMessageContainer: {
    alignItems: 'flex-end',
  },
  aiMessageContainer: {
    alignItems: 'flex-start',
  },
  glassBubble: {
    borderRadius: 20,
    overflow: 'hidden',
    maxWidth: '85%',
  },
  bubble: {
    maxWidth: '85%',
    borderRadius: 20,
    padding: 12,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.1,
    shadowRadius: 4,
    elevation: 2,
  },
  userBubble: {
    borderBottomRightRadius: 4,
  },
  aiBubble: {
    borderBottomLeftRadius: 4,
  },
  iconContainer: {
    marginBottom: 8,
  },
  contentContainer: {
    flex: 1,
  },
  messageText: {
    fontSize: 15,
    lineHeight: 22,
  },
  timestamp: {
    fontSize: 11,
    marginTop: 6,
  },
  streamingIndicator: {
    flexDirection: 'row',
    gap: 4,
    marginTop: 8,
  },
  dot: {
    width: 6,
    height: 6,
    borderRadius: 3,
    opacity: 0.6,
  },
});
