import React, { useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  Modal,
  FlatList,
  StyleSheet,
  useColorScheme,
  Platform,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { LiquidGlassView } from '@callstack/liquid-glass';
import { isLiquidGlassSupported } from '@/lib/utils/liquid-glass-helper';
import { Colors } from '@/constants/Colors';
import { useSettingsStore, AIProvider } from '@/store/settings-store';

interface ProviderSelectorProps {
  selectedProviderId: string;
  selectedModel: string;
  onSelect: (provider: AIProvider, model: string) => void;
}

export const ProviderSelector: React.FC<ProviderSelectorProps> = ({
  selectedProviderId,
  selectedModel,
  onSelect,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;
  const useLiquidGlass = Platform.OS === 'ios' && isLiquidGlassSupported();

  const [isModalVisible, setIsModalVisible] = useState(false);
  const { aiProviders } = useSettingsStore();

  const activeProviders = aiProviders.filter((p) => p.enabled);
  const selectedProvider = aiProviders.find((p) => p.id === selectedProviderId);

  const handleSelect = (provider: AIProvider) => {
    const defaultModel = provider.models[0] || 'gpt-4';
    onSelect(provider, defaultModel);
    setIsModalVisible(false);
  };

  const SelectorButton = () => (
    <TouchableOpacity
      style={[
        styles.button,
        {
          backgroundColor: colors.surface,
          borderColor: colors.border,
        },
      ]}
      onPress={() => setIsModalVisible(true)}
    >
      <View style={styles.buttonContent}>
        <Ionicons name="sparkles" size={20} color={colors.primary} />
        <View style={styles.textContainer}>
          <Text style={[styles.providerName, { color: colors.text }]}>
            {selectedProvider?.name || 'Select Provider'}
          </Text>
          <Text style={[styles.modelName, { color: colors.textSecondary }]}>
            {selectedModel}
          </Text>
        </View>
        <Ionicons name="chevron-down" size={20} color={colors.textSecondary} />
      </View>
    </TouchableOpacity>
  );

  const ProviderItem = ({ provider }: { provider: AIProvider }) => {
    const isSelected = provider.id === selectedProviderId;

    return (
      <TouchableOpacity
        style={[
          styles.providerItem,
          {
            backgroundColor: isSelected ? colors.primary + '20' : 'transparent',
            borderLeftColor: isSelected ? colors.primary : 'transparent',
          },
        ]}
        onPress={() => handleSelect(provider)}
      >
        <View style={styles.providerInfo}>
          <Text
            style={[
              styles.providerItemName,
              { color: isSelected ? colors.primary : colors.text },
            ]}
          >
            {provider.name}
          </Text>
          <Text style={[styles.providerItemModel, { color: colors.textSecondary }]}>
            {provider.models.join(', ')}
          </Text>
        </View>
        {isSelected && (
          <Ionicons name="checkmark-circle" size={24} color={colors.primary} />
        )}
      </TouchableOpacity>
    );
  };

  const ModalContent = () => (
    <View
      style={[
        styles.modalContent,
        {
          backgroundColor: colors.background,
        },
      ]}
    >
      {/* Header */}
      <View
        style={[
          styles.modalHeader,
          {
            borderBottomColor: colors.border,
          },
        ]}
      >
        <Text style={[styles.modalTitle, { color: colors.text }]}>
          Select AI Provider
        </Text>
        <TouchableOpacity onPress={() => setIsModalVisible(false)}>
          <Ionicons name="close" size={28} color={colors.textSecondary} />
        </TouchableOpacity>
      </View>

      {/* Provider List */}
      <FlatList
        data={activeProviders}
        keyExtractor={(item) => item.id}
        renderItem={({ item }) => <ProviderItem provider={item} />}
        contentContainerStyle={styles.providerList}
        showsVerticalScrollIndicator={false}
      />

      {activeProviders.length === 0 && (
        <View style={styles.emptyState}>
          <Ionicons name="information-circle" size={48} color={colors.textSecondary} />
          <Text style={[styles.emptyText, { color: colors.textSecondary }]}>
            No AI providers configured.{'\n'}
            Please add a provider in Settings.
          </Text>
        </View>
      )}
    </View>
  );

  return (
    <View>
      {useLiquidGlass ? (
        <LiquidGlassView
          style={styles.glassButton}
          effect="regular"
          interactive
          tintColor={colorScheme === 'dark' ? '#272728' : '#ffffff'}
          colorScheme={colorScheme || 'light'}
        >
          <SelectorButton />
        </LiquidGlassView>
      ) : (
        <SelectorButton />
      )}

      <Modal
        visible={isModalVisible}
        animationType="slide"
        presentationStyle="pageSheet"
        onRequestClose={() => setIsModalVisible(false)}
      >
        {useLiquidGlass ? (
          <LiquidGlassView
            style={styles.modalGlass}
            effect="regular"
            tintColor={colorScheme === 'dark' ? '#000000' : '#f2f2f7'}
            colorScheme={colorScheme || 'light'}
          >
            <ModalContent />
          </LiquidGlassView>
        ) : (
          <ModalContent />
        )}
      </Modal>
    </View>
  );
};

const styles = StyleSheet.create({
  glassButton: {
    borderRadius: 12,
    overflow: 'hidden',
  },
  button: {
    borderRadius: 12,
    borderWidth: 1,
    overflow: 'hidden',
  },
  buttonContent: {
    flexDirection: 'row',
    alignItems: 'center',
    padding: 12,
    gap: 12,
  },
  textContainer: {
    flex: 1,
  },
  providerName: {
    fontSize: 15,
    fontWeight: '600',
  },
  modelName: {
    fontSize: 12,
    marginTop: 2,
  },
  modalGlass: {
    flex: 1,
  },
  modalContent: {
    flex: 1,
  },
  modalHeader: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    padding: 16,
    borderBottomWidth: 1,
  },
  modalTitle: {
    fontSize: 20,
    fontWeight: '700',
  },
  providerList: {
    padding: 16,
  },
  providerItem: {
    flexDirection: 'row',
    alignItems: 'center',
    padding: 16,
    borderRadius: 12,
    marginBottom: 8,
    borderLeftWidth: 3,
  },
  providerInfo: {
    flex: 1,
  },
  providerItemName: {
    fontSize: 16,
    fontWeight: '600',
  },
  providerItemModel: {
    fontSize: 13,
    marginTop: 4,
  },
  emptyState: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    padding: 32,
  },
  emptyText: {
    fontSize: 15,
    textAlign: 'center',
    marginTop: 16,
    lineHeight: 22,
  },
});
