import React from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  StyleSheet,
  useColorScheme,
  Switch,
  Alert,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { Ionicons } from '@expo/vector-icons';
import { Colors } from '@/constants/Colors';
import { useSettingsStore } from '@/store/settings-store';
import { useBrowserStore } from '@/store/browser-store';
import { useAIStore } from '@/store/ai-store';
import { ProviderCard } from '@/components/settings/ProviderCard';

export default function SettingsScreen() {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;

  const {
    aiProviders,
    blockAds,
    blockTrackers,
    enableJavaScript,
    clearOnExit,
    setBlockAds,
    setBlockTrackers,
    setEnableJavaScript,
    setClearOnExit,
  } = useSettingsStore();

  const { clearHistory, clearBookmarks } = useBrowserStore();
  const { clearAllSessions } = useAIStore();

  const handleClearHistory = () => {
    Alert.alert(
      'Clear History',
      'Are you sure you want to clear your browsing history?',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Clear',
          style: 'destructive',
          onPress: () => {
            clearHistory();
            Alert.alert('Success', 'Browsing history cleared');
          },
        },
      ]
    );
  };

  const handleClearBookmarks = () => {
    Alert.alert(
      'Clear Bookmarks',
      'Are you sure you want to clear all your bookmarks?',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Clear',
          style: 'destructive',
          onPress: () => {
            clearBookmarks();
            Alert.alert('Success', 'Bookmarks cleared');
          },
        },
      ]
    );
  };

  const handleClearAIChats = () => {
    Alert.alert(
      'Clear AI Chat History',
      'Are you sure you want to clear all AI chat sessions?',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Clear',
          style: 'destructive',
          onPress: () => {
            clearAllSessions();
            Alert.alert('Success', 'AI chat history cleared');
          },
        },
      ]
    );
  };

  const SettingSection = ({ title }: { title: string }) => (
    <Text style={[styles.sectionTitle, { color: colors.textSecondary }]}>
      {title}
    </Text>
  );

  const SettingRow = ({
    icon,
    title,
    subtitle,
    value,
    onValueChange,
    showChevron = false,
    onPress,
  }: {
    icon: string;
    title: string;
    subtitle?: string;
    value?: boolean;
    onValueChange?: (value: boolean) => void;
    showChevron?: boolean;
    onPress?: () => void;
  }) => (
    <TouchableOpacity
      style={[
        styles.settingRow,
        {
          backgroundColor: colors.surface,
          borderColor: colors.border,
        },
      ]}
      onPress={onPress}
      disabled={!onPress && !onValueChange}
      activeOpacity={onPress ? 0.7 : 1}
    >
      <View style={styles.settingLeft}>
        <View style={[styles.iconContainer, { backgroundColor: colors.primary + '20' }]}>
          <Ionicons name={icon as any} size={20} color={colors.primary} />
        </View>
        <View style={styles.settingText}>
          <Text style={[styles.settingTitle, { color: colors.text }]}>
            {title}
          </Text>
          {subtitle && (
            <Text style={[styles.settingSubtitle, { color: colors.textSecondary }]}>
              {subtitle}
            </Text>
          )}
        </View>
      </View>
      {onValueChange && value !== undefined && (
        <Switch
          value={value}
          onValueChange={onValueChange}
          trackColor={{ false: colors.border, true: colors.primary }}
          thumbColor={colors.background}
        />
      )}
      {showChevron && (
        <Ionicons name="chevron-forward" size={20} color={colors.textSecondary} />
      )}
    </TouchableOpacity>
  );

  return (
    <SafeAreaView
      style={[styles.container, { backgroundColor: colors.background }]}
      edges={['bottom']}
    >
      <ScrollView
        contentContainerStyle={styles.scrollContent}
        showsVerticalScrollIndicator={false}
      >
        {/* AI Providers Section */}
        <SettingSection title="AI PROVIDERS" />
        <View style={styles.providersContainer}>
          {aiProviders.map((provider) => (
            <ProviderCard key={provider.id} provider={provider} />
          ))}
        </View>

        {/* Privacy & Security Section */}
        <SettingSection title="PRIVACY & SECURITY" />
        <View style={styles.section}>
          <SettingRow
            icon="shield-checkmark"
            title="Block Ads"
            subtitle="Block advertisements while browsing"
            value={blockAds}
            onValueChange={setBlockAds}
          />
          <SettingRow
            icon="eye-off"
            title="Block Trackers"
            subtitle="Prevent tracking scripts from running"
            value={blockTrackers}
            onValueChange={setBlockTrackers}
          />
          <SettingRow
            icon="code-slash"
            title="Enable JavaScript"
            subtitle="Allow JavaScript execution on websites"
            value={enableJavaScript}
            onValueChange={setEnableJavaScript}
          />
          <SettingRow
            icon="trash-bin"
            title="Clear on Exit"
            subtitle="Clear history when app closes"
            value={clearOnExit}
            onValueChange={setClearOnExit}
          />
        </View>

        {/* Data Management Section */}
        <SettingSection title="DATA MANAGEMENT" />
        <View style={styles.section}>
          <SettingRow
            icon="time"
            title="Clear Browsing History"
            subtitle="Remove all browsing history"
            showChevron
            onPress={handleClearHistory}
          />
          <SettingRow
            icon="bookmark"
            title="Clear Bookmarks"
            subtitle="Remove all saved bookmarks"
            showChevron
            onPress={handleClearBookmarks}
          />
          <SettingRow
            icon="chatbubbles"
            title="Clear AI Chat History"
            subtitle="Remove all AI chat sessions"
            showChevron
            onPress={handleClearAIChats}
          />
        </View>

        {/* About Section */}
        <SettingSection title="ABOUT" />
        <View style={styles.section}>
          <View
            style={[
              styles.aboutCard,
              {
                backgroundColor: colors.surface,
                borderColor: colors.border,
              },
            ]}
          >
            <Text style={[styles.appName, { color: colors.text }]}>
              BlockBrowser
            </Text>
            <Text style={[styles.version, { color: colors.textSecondary }]}>
              Version 1.0.0
            </Text>
            <Text style={[styles.description, { color: colors.textSecondary }]}>
              AI-powered mobile browser with privacy-first features
            </Text>
          </View>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  scrollContent: {
    padding: 16,
    paddingBottom: 32,
  },
  sectionTitle: {
    fontSize: 13,
    fontWeight: '600',
    marginTop: 24,
    marginBottom: 8,
    marginLeft: 4,
    letterSpacing: 0.5,
  },
  section: {
    gap: 8,
  },
  providersContainer: {
    gap: 12,
  },
  settingRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    padding: 14,
    borderRadius: 12,
    borderWidth: 1,
  },
  settingLeft: {
    flex: 1,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 12,
  },
  iconContainer: {
    width: 36,
    height: 36,
    borderRadius: 8,
    alignItems: 'center',
    justifyContent: 'center',
  },
  settingText: {
    flex: 1,
  },
  settingTitle: {
    fontSize: 15,
    fontWeight: '600',
  },
  settingSubtitle: {
    fontSize: 13,
    marginTop: 2,
  },
  aboutCard: {
    padding: 20,
    borderRadius: 12,
    borderWidth: 1,
    alignItems: 'center',
  },
  appName: {
    fontSize: 24,
    fontWeight: '700',
  },
  version: {
    fontSize: 14,
    marginTop: 4,
  },
  description: {
    fontSize: 14,
    marginTop: 12,
    textAlign: 'center',
    lineHeight: 20,
  },
});
