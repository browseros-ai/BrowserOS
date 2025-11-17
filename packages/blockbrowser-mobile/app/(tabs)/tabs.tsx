import React from 'react';
import {
  View,
  Text,
  FlatList,
  TouchableOpacity,
  StyleSheet,
  useColorScheme,
  Platform,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { Ionicons } from '@expo/vector-icons';
import { LiquidGlassView, isLiquidGlassSupported } from '@callstack/liquid-glass';
import { Colors } from '@/constants/Colors';
import { useBrowserStore } from '@/store/browser-store';
import { TabCard } from '@/components/browser/TabCard';

export default function TabsScreen() {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;
  const useLiquidGlass = Platform.OS === 'ios' && isLiquidGlassSupported();
  const router = useRouter();

  const { tabs, activeTabId, addTab, closeTab, setActiveTab } = useBrowserStore();

  const handleSelectTab = (tabId: string) => {
    setActiveTab(tabId);
    router.push('/(tabs)');
  };

  const handleCloseTab = (tabId: string) => {
    closeTab(tabId);
  };

  const handleNewTab = () => {
    const newTabId = addTab('https://www.google.com');
    setActiveTab(newTabId);
    router.push('/(tabs)');
  };

  const NewTabButton = () => (
    <TouchableOpacity
      style={[
        styles.newTabButton,
        { backgroundColor: colors.primary },
      ]}
      onPress={handleNewTab}
    >
      <Ionicons name="add" size={24} color={colors.background} />
      <Text style={[styles.newTabButtonText, { color: colors.background }]}>
        New Tab
      </Text>
    </TouchableOpacity>
  );

  const EmptyState = () => (
    <View style={styles.emptyContainer}>
      <Ionicons name="browsers-outline" size={64} color={colors.textSecondary} />
      <Text style={[styles.emptyTitle, { color: colors.text }]}>
        No Open Tabs
      </Text>
      <Text style={[styles.emptySubtitle, { color: colors.textSecondary }]}>
        Create a new tab to start browsing
      </Text>
      <View style={styles.emptyButton}>
        <NewTabButton />
      </View>
    </View>
  );

  return (
    <SafeAreaView
      style={[styles.container, { backgroundColor: colors.background }]}
      edges={['bottom']}
    >
      {tabs.length === 0 ? (
        <EmptyState />
      ) : (
        <>
          {/* Tab List */}
          <FlatList
            data={tabs}
            keyExtractor={(item) => item.id}
            renderItem={({ item }) => (
              <TabCard
                tab={item}
                isActive={item.id === activeTabId}
                onSelect={() => handleSelectTab(item.id)}
                onClose={() => handleCloseTab(item.id)}
              />
            )}
            contentContainerStyle={styles.tabList}
            showsVerticalScrollIndicator={false}
          />

          {/* New Tab Button */}
          <View
            style={[
              styles.footer,
              {
                backgroundColor: colors.surface,
                borderTopColor: colors.border,
              },
            ]}
          >
            {useLiquidGlass ? (
              <LiquidGlassView
                style={styles.glassButton}
                effect="regular"
                interactive
                tintColor={colorScheme === 'dark' ? '#272728' : '#ffffff'}
                colorScheme={colorScheme || 'light'}
              >
                <NewTabButton />
              </LiquidGlassView>
            ) : (
              <NewTabButton />
            )}
          </View>
        </>
      )}
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  tabList: {
    padding: 16,
  },
  footer: {
    padding: 16,
    borderTopWidth: 1,
  },
  glassButton: {
    borderRadius: 12,
    overflow: 'hidden',
  },
  newTabButton: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    paddingVertical: 14,
    borderRadius: 12,
    gap: 8,
  },
  newTabButtonText: {
    fontSize: 16,
    fontWeight: '600',
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
  emptyButton: {
    marginTop: 32,
    width: '100%',
    maxWidth: 300,
  },
});
