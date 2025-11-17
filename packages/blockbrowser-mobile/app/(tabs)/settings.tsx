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
import { LinearGradient } from 'expo-linear-gradient';
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
          <LinearGradient
            colors={['#667eea', '#764ba2']}
            start={{ x: 0, y: 0 }}
            end={{ x: 1, y: 1 }}
            style={styles.aboutCard}
          >
            <View style={styles.aboutLogo}>
              <Text style={styles.aboutLogoText}>B</Text>
            </View>
            <Text style={styles.appName}>BlockBrowser</Text>
            <Text style={styles.version}>Version 1.0.0</Text>
            <Text style={styles.description}>
              AI-powered mobile browser with privacy-first features
            </Text>
            <View style={styles.featuresList}>
              <View style={styles.featureItem}>
                <Ionicons name="shield-checkmark" size={16} color="#fff" />
                <Text style={styles.featureText}>Privacy-first</Text>
              </View>
              <View style={styles.featureItem}>
                <Ionicons name="sparkles" size={16} color="#fff" />
                <Text style={styles.featureText}>AI-powered</Text>
              </View>
              <View style={styles.featureItem}>
                <Ionicons name="flash" size={16} color="#fff" />
                <Text style={styles.featureText}>Fast & secure</Text>
              </View>
            </View>
          </LinearGradient>
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
    padding: 24,
    borderRadius: 20,
    alignItems: 'center',
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.3,
    shadowRadius: 8,
    elevation: 8,
  },
  aboutLogo: {
    width: 64,
    height: 64,
    borderRadius: 16,
    backgroundColor: 'rgba(255, 255, 255, 0.2)',
    alignItems: 'center',
    justifyContent: 'center',
    borderWidth: 2,
    borderColor: 'rgba(255, 255, 255, 0.3)',
    marginBottom: 16,
  },
  aboutLogoText: {
    fontSize: 36,
    fontWeight: '900',
    color: '#fff',
  },
  appName: {
    fontSize: 28,
    fontWeight: '800',
    color: '#fff',
    letterSpacing: 0.5,
  },
  version: {
    fontSize: 14,
    color: 'rgba(255, 255, 255, 0.8)',
    marginTop: 6,
  },
  description: {
    fontSize: 15,
    color: 'rgba(255, 255, 255, 0.95)',
    marginTop: 12,
    textAlign: 'center',
    lineHeight: 22,
  },
  featuresList: {
    flexDirection: 'row',
    marginTop: 20,
    gap: 16,
  },
  featureItem: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    backgroundColor: 'rgba(255, 255, 255, 0.15)',
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderRadius: 16,
  },
  featureText: {
    color: '#fff',
    fontSize: 13,
    fontWeight: '600',
  },
});
