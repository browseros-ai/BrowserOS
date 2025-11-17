import React, { useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  StyleSheet,
  useColorScheme,
  Switch,
  Platform,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { LiquidGlassView, isLiquidGlassSupported } from '@callstack/liquid-glass';
import { Colors } from '@/constants/Colors';
import { AIProvider, useSettingsStore } from '@/store/settings-store';
import { ApiKeyInput } from './ApiKeyInput';

interface ProviderCardProps {
  provider: AIProvider;
}

export const ProviderCard: React.FC<ProviderCardProps> = ({ provider }) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;
  const useLiquidGlass = Platform.OS === 'ios' && isLiquidGlassSupported();

  const [isExpanded, setIsExpanded] = useState(false);
  const { updateProvider } = useSettingsStore();

  const handleToggle = (enabled: boolean) => {
    updateProvider(provider.id, { enabled });
  };

  const handleApiKeySave = (apiKey: string) => {
    // API key is saved securely via ApiKeyInput component
    // Just close the expansion
    setIsExpanded(false);
  };

  const getProviderIcon = (type: string): string => {
    switch (type) {
      case 'openai':
      case 'openai_compatible':
        return 'logo-openai';
      case 'anthropic':
        return 'cube';
      case 'google_gemini':
        return 'logo-google';
      case 'ollama':
        return 'server';
      case 'blockbrowser':
        return 'sparkles';
      default:
        return 'flash';
    }
  };

  const CardContent = () => (
    <View
      style={[
        styles.card,
        {
          backgroundColor: colors.surface,
          borderColor: provider.enabled ? colors.primary : colors.border,
        },
      ]}
    >
      {/* Header */}
      <TouchableOpacity
        style={styles.header}
        onPress={() => setIsExpanded(!isExpanded)}
        activeOpacity={0.7}
      >
        <View style={styles.headerLeft}>
          <View
            style={[
              styles.iconContainer,
              {
                backgroundColor: provider.enabled
                  ? colors.primary + '20'
                  : colors.border + '40',
              },
            ]}
          >
            <Ionicons
              name={getProviderIcon(provider.type) as any}
              size={24}
              color={provider.enabled ? colors.primary : colors.textSecondary}
            />
          </View>
          <View style={styles.providerInfo}>
            <Text style={[styles.providerName, { color: colors.text }]}>
              {provider.name}
            </Text>
            <Text style={[styles.providerModels, { color: colors.textSecondary }]}>
              {provider.models.join(', ')}
            </Text>
          </View>
        </View>
        <View style={styles.headerRight}>
          <Switch
            value={provider.enabled}
            onValueChange={handleToggle}
            trackColor={{ false: colors.border, true: colors.primary }}
            thumbColor={colors.background}
          />
          <Ionicons
            name={isExpanded ? 'chevron-up' : 'chevron-down'}
            size={20}
            color={colors.textSecondary}
            style={styles.chevron}
          />
        </View>
      </TouchableOpacity>

      {/* Expanded Content */}
      {isExpanded && (
        <View style={styles.expandedContent}>
          {/* API Configuration */}
          {provider.type !== 'blockbrowser' && (
            <>
              <View style={[styles.divider, { backgroundColor: colors.border }]} />

              {/* API Key Input */}
              <View style={styles.configSection}>
                <Text style={[styles.sectionLabel, { color: colors.text }]}>
                  API Key
                </Text>
                <ApiKeyInput
                  providerId={provider.id}
                  onSave={handleApiKeySave}
                />
              </View>

              {/* Base URL for custom providers */}
              {(provider.type === 'openai_compatible' ||
                provider.type === 'ollama' ||
                provider.type === 'custom') && (
                <View style={styles.configSection}>
                  <Text style={[styles.sectionLabel, { color: colors.text }]}>
                    Base URL
                  </Text>
                  <Text style={[styles.baseUrl, { color: colors.textSecondary }]}>
                    {provider.baseUrl || 'Not configured'}
                  </Text>
                </View>
              )}
            </>
          )}

          {/* Provider Description */}
          <View style={styles.descriptionSection}>
            <Text style={[styles.description, { color: colors.textSecondary }]}>
              {getProviderDescription(provider.type)}
            </Text>
          </View>
        </View>
      )}
    </View>
  );

  if (useLiquidGlass) {
    return (
      <LiquidGlassView
        style={styles.glassCard}
        effect="regular"
        interactive
        tintColor={colorScheme === 'dark' ? '#272728' : '#ffffff'}
        colorScheme={colorScheme || 'light'}
      >
        <CardContent />
      </LiquidGlassView>
    );
  }

  return <CardContent />;
};

const getProviderDescription = (type: string): string => {
  switch (type) {
    case 'openai':
      return 'Access to GPT-4, GPT-3.5 and other OpenAI models. Requires an OpenAI API key.';
    case 'anthropic':
      return 'Access to Claude models from Anthropic. Requires an Anthropic API key.';
    case 'google_gemini':
      return 'Access to Google Gemini models. Requires a Google AI API key.';
    case 'openrouter':
      return 'Access to multiple AI models through OpenRouter. Requires an OpenRouter API key.';
    case 'ollama':
      return 'Run AI models locally with Ollama. Requires Ollama running on your network.';
    case 'blockbrowser':
      return 'Built-in BlockBrowser AI assistant. No API key required.';
    default:
      return 'Custom AI provider configuration.';
  }
};

const styles = StyleSheet.create({
  glassCard: {
    borderRadius: 16,
    overflow: 'hidden',
  },
  card: {
    borderRadius: 16,
    borderWidth: 2,
    overflow: 'hidden',
  },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    padding: 14,
  },
  headerLeft: {
    flex: 1,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 12,
  },
  iconContainer: {
    width: 48,
    height: 48,
    borderRadius: 12,
    alignItems: 'center',
    justifyContent: 'center',
  },
  providerInfo: {
    flex: 1,
  },
  providerName: {
    fontSize: 16,
    fontWeight: '600',
  },
  providerModels: {
    fontSize: 13,
    marginTop: 2,
  },
  headerRight: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
  },
  chevron: {
    marginLeft: 4,
  },
  expandedContent: {
    paddingBottom: 14,
  },
  divider: {
    height: 1,
    marginHorizontal: 14,
    marginBottom: 14,
  },
  configSection: {
    paddingHorizontal: 14,
    marginBottom: 14,
  },
  sectionLabel: {
    fontSize: 14,
    fontWeight: '600',
    marginBottom: 8,
  },
  baseUrl: {
    fontSize: 13,
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
  },
  descriptionSection: {
    paddingHorizontal: 14,
    marginTop: 4,
  },
  description: {
    fontSize: 13,
    lineHeight: 18,
  },
});
