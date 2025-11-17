import React, { useState, useEffect } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  StyleSheet,
  useColorScheme,
  Platform,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { Ionicons } from '@expo/vector-icons';
import { LiquidGlassView, isLiquidGlassSupported } from '@callstack/liquid-glass';
import { Colors } from '@/constants/Colors';
import { useAIStore } from '@/store/ai-store';
import { useSettingsStore } from '@/store/settings-store';
import { ChatInterface } from '@/components/ai/ChatInterface';
import { ProviderSelector } from '@/components/ai/ProviderSelector';

export default function AIScreen() {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;
  const useLiquidGlass = Platform.OS === 'ios' && isLiquidGlassSupported();

  const { sessions, activeSessionId, createSession, setActiveSession } = useAIStore();
  const { aiProviders } = useSettingsStore();

  const [selectedProviderId, setSelectedProviderId] = useState<string>('');
  const [selectedModel, setSelectedModel] = useState<string>('');

  const activeSession = sessions.find((s) => s.id === activeSessionId);

  useEffect(() => {
    // Initialize with first enabled provider
    const firstProvider = aiProviders.find((p) => p.enabled);
    if (firstProvider && !selectedProviderId) {
      setSelectedProviderId(firstProvider.id);
      setSelectedModel(firstProvider.models[0] || '');
    }
  }, [aiProviders]);

  const handleNewChat = () => {
    if (!selectedProviderId) return;

    const provider = aiProviders.find((p) => p.id === selectedProviderId);
    if (!provider) return;

    const sessionId = createSession(
      selectedProviderId,
      provider.type,
      selectedModel
    );
    setActiveSession(sessionId);
  };

  const handleProviderSelect = (provider: any, model: string) => {
    setSelectedProviderId(provider.id);
    setSelectedModel(model);
  };

  const EmptyState = () => (
    <View style={styles.emptyContainer}>
      <Ionicons name="sparkles" size={64} color={colors.primary} />
      <Text style={[styles.emptyTitle, { color: colors.text }]}>
        Welcome to BlockBrowser AI
      </Text>
      <Text style={[styles.emptySubtitle, { color: colors.textSecondary }]}>
        Start a conversation with your AI assistant
      </Text>

      <View style={styles.providerSelectorContainer}>
        <ProviderSelector
          selectedProviderId={selectedProviderId}
          selectedModel={selectedModel}
          onSelect={handleProviderSelect}
        />
      </View>

      <TouchableOpacity
        style={[styles.newChatButton, { backgroundColor: colors.primary }]}
        onPress={handleNewChat}
        disabled={!selectedProviderId}
      >
        <Ionicons name="add-circle" size={24} color={colors.background} />
        <Text style={[styles.newChatButtonText, { color: colors.background }]}>
          Start New Chat
        </Text>
      </TouchableOpacity>
    </View>
  );

  return (
    <SafeAreaView
      style={[styles.container, { backgroundColor: colors.background }]}
      edges={['bottom']}
    >
      {activeSession ? (
        <>
          {/* Header with Provider Selector */}
          <View
            style={[
              styles.header,
              {
                backgroundColor: colors.surface,
                borderBottomColor: colors.border,
              },
            ]}
          >
            <ProviderSelector
              selectedProviderId={activeSession.providerId}
              selectedModel={activeSession.model}
              onSelect={(provider, model) => {
                // Optionally allow switching provider mid-conversation
                // For now, just update the selection
                setSelectedProviderId(provider.id);
                setSelectedModel(model);
              }}
            />
            <TouchableOpacity
              style={styles.newChatIcon}
              onPress={handleNewChat}
            >
              <Ionicons name="create-outline" size={24} color={colors.primary} />
            </TouchableOpacity>
          </View>

          {/* Chat Interface */}
          <ChatInterface sessionId={activeSession.id} />
        </>
      ) : (
        <EmptyState />
      )}
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    padding: 12,
    borderBottomWidth: 1,
    gap: 12,
  },
  newChatIcon: {
    padding: 8,
  },
  emptyContainer: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    padding: 32,
  },
  emptyTitle: {
    fontSize: 24,
    fontWeight: '700',
    marginTop: 24,
    textAlign: 'center',
  },
  emptySubtitle: {
    fontSize: 16,
    marginTop: 8,
    textAlign: 'center',
    lineHeight: 24,
  },
  providerSelectorContainer: {
    width: '100%',
    marginTop: 32,
    marginBottom: 16,
  },
  newChatButton: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 24,
    paddingVertical: 14,
    borderRadius: 12,
    gap: 8,
  },
  newChatButtonText: {
    fontSize: 16,
    fontWeight: '600',
  },
});
