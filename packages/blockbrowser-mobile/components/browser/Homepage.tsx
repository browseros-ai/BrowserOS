import React from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  StyleSheet,
  useColorScheme,
  FlatList,
  Image,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { Colors } from '@/constants/Colors';
import { useBrowserStore } from '@/store/browser-store';

interface HomepageProps {
  onNavigate: (url: string) => void;
}

export const Homepage: React.FC<HomepageProps> = ({ onNavigate }) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;
  const { bookmarks, history } = useBrowserStore();

  // Quick access shortcuts
  const shortcuts = [
    { id: '1', name: 'Google', url: 'https://www.google.com', icon: 'search' },
    { id: '2', name: 'YouTube', url: 'https://www.youtube.com', icon: 'logo-youtube' },
    { id: '3', name: 'Twitter', url: 'https://twitter.com', icon: 'logo-twitter' },
    { id: '4', name: 'GitHub', url: 'https://github.com', icon: 'logo-github' },
    { id: '5', name: 'Reddit', url: 'https://reddit.com', icon: 'logo-reddit' },
    { id: '6', name: 'Wikipedia', url: 'https://wikipedia.org', icon: 'book' },
  ];

  const recentHistory = history.slice(0, 10);

  return (
    <View style={[styles.container, { backgroundColor: colors.background }]}>
      {/* Logo/Header */}
      <View style={styles.header}>
        <Ionicons name="browsers" size={64} color={colors.primary} />
        <Text style={[styles.title, { color: colors.text }]}>BlockBrowser</Text>
        <Text style={[styles.subtitle, { color: colors.textSecondary }]}>
          Privacy-first mobile browsing
        </Text>
      </View>

      {/* Quick Access Shortcuts */}
      <View style={styles.section}>
        <Text style={[styles.sectionTitle, { color: colors.text }]}>
          Quick Access
        </Text>
        <View style={styles.shortcutsGrid}>
          {shortcuts.map((shortcut) => (
            <TouchableOpacity
              key={shortcut.id}
              style={[
                styles.shortcutCard,
                {
                  backgroundColor: colors.surface,
                  borderColor: colors.border,
                },
              ]}
              onPress={() => onNavigate(shortcut.url)}
              activeOpacity={0.7}
            >
              <View
                style={[
                  styles.shortcutIcon,
                  { backgroundColor: colors.primary + '20' },
                ]}
              >
                <Ionicons
                  name={shortcut.icon as any}
                  size={28}
                  color={colors.primary}
                />
              </View>
              <Text style={[styles.shortcutName, { color: colors.text }]}>
                {shortcut.name}
              </Text>
            </TouchableOpacity>
          ))}
        </View>
      </View>

      {/* Bookmarks */}
      {bookmarks.length > 0 && (
        <View style={styles.section}>
          <Text style={[styles.sectionTitle, { color: colors.text }]}>
            Bookmarks
          </Text>
          <FlatList
            data={bookmarks.slice(0, 5)}
            horizontal
            showsHorizontalScrollIndicator={false}
            keyExtractor={(item) => item.id}
            renderItem={({ item }) => (
              <TouchableOpacity
                style={[
                  styles.bookmarkCard,
                  {
                    backgroundColor: colors.surface,
                    borderColor: colors.border,
                  },
                ]}
                onPress={() => onNavigate(item.url)}
              >
                {item.favicon ? (
                  <Image
                    source={{ uri: item.favicon }}
                    style={styles.favicon}
                  />
                ) : (
                  <Ionicons name="bookmark" size={20} color={colors.primary} />
                )}
                <Text
                  style={[styles.bookmarkTitle, { color: colors.text }]}
                  numberOfLines={1}
                >
                  {item.title}
                </Text>
              </TouchableOpacity>
            )}
          />
        </View>
      )}

      {/* Recent History */}
      {recentHistory.length > 0 && (
        <View style={styles.section}>
          <Text style={[styles.sectionTitle, { color: colors.text }]}>
            Recently Visited
          </Text>
          <FlatList
            data={recentHistory}
            keyExtractor={(item) => item.id}
            renderItem={({ item }) => (
              <TouchableOpacity
                style={[
                  styles.historyItem,
                  {
                    backgroundColor: colors.surface,
                    borderBottomColor: colors.border,
                  },
                ]}
                onPress={() => onNavigate(item.url)}
              >
                <Ionicons name="time" size={20} color={colors.textSecondary} />
                <View style={styles.historyText}>
                  <Text
                    style={[styles.historyTitle, { color: colors.text }]}
                    numberOfLines={1}
                  >
                    {item.title}
                  </Text>
                  <Text
                    style={[styles.historyUrl, { color: colors.textSecondary }]}
                    numberOfLines={1}
                  >
                    {item.url}
                  </Text>
                </View>
              </TouchableOpacity>
            )}
          />
        </View>
      )}
    </View>
  );
};

const styles = StyleSheet.create({
  container: {
    flex: 1,
    padding: 16,
  },
  header: {
    alignItems: 'center',
    paddingVertical: 32,
  },
  title: {
    fontSize: 32,
    fontWeight: '700',
    marginTop: 16,
  },
  subtitle: {
    fontSize: 16,
    marginTop: 8,
  },
  section: {
    marginTop: 24,
  },
  sectionTitle: {
    fontSize: 20,
    fontWeight: '600',
    marginBottom: 16,
  },
  shortcutsGrid: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: 12,
  },
  shortcutCard: {
    width: '30%',
    aspectRatio: 1,
    borderRadius: 16,
    borderWidth: 1,
    padding: 16,
    alignItems: 'center',
    justifyContent: 'center',
    gap: 8,
  },
  shortcutIcon: {
    width: 56,
    height: 56,
    borderRadius: 28,
    alignItems: 'center',
    justifyContent: 'center',
  },
  shortcutName: {
    fontSize: 13,
    fontWeight: '500',
    textAlign: 'center',
  },
  bookmarkCard: {
    width: 120,
    padding: 12,
    borderRadius: 12,
    borderWidth: 1,
    marginRight: 12,
    alignItems: 'center',
    gap: 8,
  },
  favicon: {
    width: 24,
    height: 24,
    borderRadius: 4,
  },
  bookmarkTitle: {
    fontSize: 13,
    fontWeight: '500',
    textAlign: 'center',
  },
  historyItem: {
    flexDirection: 'row',
    alignItems: 'center',
    padding: 12,
    borderRadius: 12,
    marginBottom: 8,
    gap: 12,
    borderBottomWidth: 1,
  },
  historyText: {
    flex: 1,
  },
  historyTitle: {
    fontSize: 15,
    fontWeight: '500',
  },
  historyUrl: {
    fontSize: 13,
    marginTop: 2,
  },
});
