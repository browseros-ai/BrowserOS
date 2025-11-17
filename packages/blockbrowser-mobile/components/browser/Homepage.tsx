import React from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  StyleSheet,
  useColorScheme,
  FlatList,
  Image,
  ScrollView,
} from 'react-native';
import { LinearGradient } from 'expo-linear-gradient';
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

  // Quick access shortcuts with individual colors
  const shortcuts = [
    { id: '1', name: 'Google', url: 'https://www.google.com', icon: 'search', color: '#4285F4' },
    { id: '2', name: 'YouTube', url: 'https://www.youtube.com', icon: 'logo-youtube', color: '#FF0000' },
    { id: '3', name: 'Twitter', url: 'https://twitter.com', icon: 'logo-twitter', color: '#1DA1F2' },
    { id: '4', name: 'GitHub', url: 'https://github.com', icon: 'logo-github', color: '#333' },
    { id: '5', name: 'Reddit', url: 'https://reddit.com', icon: 'logo-reddit', color: '#FF4500' },
    { id: '6', name: 'Wikipedia', url: 'https://wikipedia.org', icon: 'book', color: '#000' },
  ];

  const recentHistory = history.slice(0, 10);

  return (
    <ScrollView
      style={[styles.container, { backgroundColor: colors.background }]}
      showsVerticalScrollIndicator={false}
    >
      {/* Gradient Header with Logo */}
      <LinearGradient
        colors={['#667eea', '#764ba2']}
        start={{ x: 0, y: 0 }}
        end={{ x: 1, y: 1 }}
        style={styles.gradientHeader}
      >
        <View style={styles.logoContainer}>
          <View style={styles.logoPlaceholder}>
            <Text style={styles.logoText}>B</Text>
          </View>
          <Text style={styles.brandTitle}>BlockBrowser</Text>
          <Text style={styles.brandSubtitle}>
            Privacy-first mobile browsing
          </Text>
        </View>
      </LinearGradient>

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
                  { backgroundColor: shortcut.color + '15' },
                ]}
              >
                <Ionicons
                  name={shortcut.icon as any}
                  size={28}
                  color={shortcut.color}
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
    </ScrollView>
  );
};

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  gradientHeader: {
    paddingTop: 60,
    paddingBottom: 40,
    paddingHorizontal: 24,
    borderBottomLeftRadius: 32,
    borderBottomRightRadius: 32,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.3,
    shadowRadius: 8,
    elevation: 8,
  },
  logoContainer: {
    alignItems: 'center',
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
  brandTitle: {
    fontSize: 32,
    fontWeight: '800',
    color: '#fff',
    marginTop: 16,
    letterSpacing: 0.5,
  },
  brandSubtitle: {
    fontSize: 16,
    color: 'rgba(255, 255, 255, 0.9)',
    marginTop: 8,
  },
  section: {
    marginTop: 24,
    paddingHorizontal: 16,
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
    borderRadius: 20,
    borderWidth: 1,
    padding: 16,
    alignItems: 'center',
    justifyContent: 'center',
    gap: 8,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.1,
    shadowRadius: 4,
    elevation: 3,
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
