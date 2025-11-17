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
import { LinearGradient } from 'expo-linear-gradient';
import { Ionicons } from '@expo/vector-icons';
import { LiquidGlassView } from '@callstack/liquid-glass';
import { isLiquidGlassSupported } from '@/lib/utils/liquid-glass-helper';
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
      {/* Gradient Header with Logo */}
      <LinearGradient
        colors={['#667eea', '#764ba2']}
        start={{ x: 0, y: 0 }}
        end={{ x: 1, y: 1 }}
        style={styles.emptyGradientHeader}
      >
        <View style={styles.logoPlaceholder}>
          <Text style={styles.logoText}>B</Text>
        </View>
        <View style={styles.sparklesIcon}>
          <Ionicons name="sparkles" size={32} color="#fff" />
        </View>
        <Text style={styles.emptyHeaderTitle}>BlockBrowser AI</Text>
        <Text style={styles.emptyHeaderSubtitle}>
          Powered by multiple AI providers
        </Text>
      </LinearGradient>

      <View style={styles.emptyContent}>
        <Text style={[styles.emptyTitle, { color: colors.text }]}>
          Start a conversation
        </Text>
        <Text style={[styles.emptySubtitle, { color: colors.textSecondary }]}>
          Choose your AI provider and model to begin chatting
        </Text>

        <View style={styles.providerSelectorContainer}>
          <ProviderSelector
            selectedProviderId={selectedProviderId}
            selectedModel={selectedModel}
            onSelect={handleProviderSelect}
          />
        </View>

        <TouchableOpacity
          style={[
            styles.newChatButton,
            { backgroundColor: selectedProviderId ? colors.primary : colors.border },
          ]}
          onPress={handleNewChat}
          disabled={!selectedProviderId}
        >
          <Ionicons
            name="add-circle"
            size={24}
            color={selectedProviderId ? colors.background : colors.textSecondary}
          />
          <Text
            style={[
              styles.newChatButtonText,
              { color: selectedProviderId ? colors.background : colors.textSecondary },
            ]}
          >
            Start New Chat
          </Text>
        </TouchableOpacity>
      </View>
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
  },
  emptyGradientHeader: {
    paddingTop: 60,
    paddingBottom: 40,
    paddingHorizontal: 24,
    alignItems: 'center',
    borderBottomLeftRadius: 32,
    borderBottomRightRadius: 32,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.3,
    shadowRadius: 8,
    elevation: 8,
  },
  logoPlaceholder: {
    width: 80,
    height: 80,
    borderRadius: 20,
    backgroundColor: 'rgba(255, 255, 255, 0.2)',
    alignItems: 'center',
    justifyContent: 'center',
    borderWidth: 2,
    borderColor: 'rgba(255, 255, 255, 0.3)',
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.2,
    shadowRadius: 8,
    elevation: 4,
  },
  logoText: {
    fontSize: 48,
    fontWeight: '900',
    color: '#fff',
  },
  sparklesIcon: {
    marginTop: 16,
    padding: 12,
    backgroundColor: 'rgba(255, 255, 255, 0.2)',
    borderRadius: 50,
  },
  emptyHeaderTitle: {
    fontSize: 28,
    fontWeight: '800',
    color: '#fff',
    marginTop: 16,
    letterSpacing: 0.5,
  },
  emptyHeaderSubtitle: {
    fontSize: 15,
    color: 'rgba(255, 255, 255, 0.9)',
    marginTop: 8,
  },
  emptyContent: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    padding: 32,
  },
  emptyTitle: {
    fontSize: 22,
    fontWeight: '700',
    textAlign: 'center',
  },
  emptySubtitle: {
    fontSize: 15,
    marginTop: 8,
    textAlign: 'center',
    lineHeight: 22,
  },
  providerSelectorContainer: {
    width: '100%',
    marginTop: 32,
    marginBottom: 16,
  },
  newChatButton: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 28,
    paddingVertical: 16,
    borderRadius: 16,
    gap: 8,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.15,
    shadowRadius: 4,
    elevation: 4,
  },
  newChatButtonText: {
    fontSize: 17,
    fontWeight: '700',
  },
});
