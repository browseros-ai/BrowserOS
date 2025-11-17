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
import { LinearGradient } from 'expo-linear-gradient';
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
      <LinearGradient
        colors={['#667eea', '#764ba2']}
        start={{ x: 0, y: 0 }}
        end={{ x: 1, y: 1 }}
        style={styles.emptyGradient}
      >
        <View style={styles.emptyLogoPlaceholder}>
          <Text style={styles.emptyLogoText}>B</Text>
        </View>
        <Ionicons name="browsers-outline" size={48} color="#fff" />
        <Text style={styles.emptyGradientTitle}>No Open Tabs</Text>
        <Text style={styles.emptyGradientSubtitle}>
          Create a new tab to start browsing the web
        </Text>
      </LinearGradient>
      <View style={styles.emptyContent}>
        <Text style={[styles.emptyTitle, { color: colors.text }]}>
          Get Started
        </Text>
        <Text style={[styles.emptySubtitle, { color: colors.textSecondary }]}>
          Open a new tab and explore
        </Text>
        <View style={styles.emptyButton}>
          <NewTabButton />
        </View>
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
  },
  emptyGradient: {
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
  emptyLogoPlaceholder: {
    width: 72,
    height: 72,
    borderRadius: 18,
    backgroundColor: 'rgba(255, 255, 255, 0.2)',
    alignItems: 'center',
    justifyContent: 'center',
    borderWidth: 2,
    borderColor: 'rgba(255, 255, 255, 0.3)',
    marginBottom: 16,
  },
  emptyLogoText: {
    fontSize: 42,
    fontWeight: '900',
    color: '#fff',
  },
  emptyGradientTitle: {
    fontSize: 26,
    fontWeight: '800',
    color: '#fff',
    marginTop: 16,
    letterSpacing: 0.5,
  },
  emptyGradientSubtitle: {
    fontSize: 15,
    color: 'rgba(255, 255, 255, 0.9)',
    marginTop: 8,
    textAlign: 'center',
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
  emptyButton: {
    marginTop: 32,
    width: '100%',
    maxWidth: 300,
  },
});
