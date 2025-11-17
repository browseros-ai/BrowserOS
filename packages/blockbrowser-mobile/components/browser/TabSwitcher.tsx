/**
 * TabSwitcher - Premium card-style tab switcher
 * Swipe to close, smooth animations
 */

import React, { useRef, useState } from 'react';
import {
  View,
  Text,
  Animated,
  TouchableOpacity,
  Dimensions,
  StyleSheet,
  useColorScheme,
  ScrollView,
  Image,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { LinearGradient } from 'expo-linear-gradient';
import { Colors } from '@/constants/Colors';
import { HapticFeedback } from '@/lib/utils/haptics';
import { SwipeableRow } from '../common/SwipeableRow';
import { AnimatedButton } from '../common/AnimatedButton';

const SCREEN_WIDTH = Dimensions.get('window').width;
const CARD_WIDTH = SCREEN_WIDTH - 48;
const CARD_HEIGHT = 400;

interface Tab {
  id: string;
  title: string;
  url: string;
  favicon?: string;
  screenshot?: string;
}

interface TabSwitcherProps {
  tabs: Tab[];
  activeTabId: string;
  onTabSelect: (tabId: string) => void;
  onTabClose: (tabId: string) => void;
  onNewTab: () => void;
  visible: boolean;
  onClose: () => void;
}

export const TabSwitcher: React.FC<TabSwitcherProps> = ({
  tabs,
  activeTabId,
  onTabSelect,
  onTabClose,
  onNewTab,
  visible,
  onClose,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;

  const opacity = useRef(new Animated.Value(0)).current;
  const scale = useRef(new Animated.Value(0.9)).current;

  React.useEffect(() => {
    if (visible) {
      // Enter animation
      Animated.parallel([
        Animated.timing(opacity, {
          toValue: 1,
          duration: 200,
          useNativeDriver: true,
        }),
        Animated.spring(scale, {
          toValue: 1,
          tension: 300,
          friction: 30,
          useNativeDriver: true,
        }),
      ]).start();
    } else {
      // Exit animation
      Animated.parallel([
        Animated.timing(opacity, {
          toValue: 0,
          duration: 150,
          useNativeDriver: true,
        }),
        Animated.timing(scale, {
          toValue: 0.9,
          duration: 150,
          useNativeDriver: true,
        }),
      ]).start();
    }
  }, [visible]);

  if (!visible) return null;

  const handleTabSelect = (tabId: string) => {
    HapticFeedback.medium();
    onTabSelect(tabId);
    onClose();
  };

  const handleTabClose = (tabId: string) => {
    HapticFeedback.heavy();
    onTabClose(tabId);
  };

  const handleNewTab = () => {
    HapticFeedback.success();
    onNewTab();
    onClose();
  };

  return (
    <Animated.View
      style={[
        styles.overlay,
        {
          backgroundColor: colors.background,
          opacity,
        },
      ]}
    >
      <Animated.View
        style={[
          styles.container,
          {
            transform: [{ scale }],
          },
        ]}
      >
        {/* Header */}
        <View style={styles.header}>
          <Text style={[styles.headerTitle, { color: colors.text }]}>
            {tabs.length} {tabs.length === 1 ? 'Tab' : 'Tabs'}
          </Text>
          <TouchableOpacity onPress={onClose}>
            <Ionicons name="close" size={28} color={colors.text} />
          </TouchableOpacity>
        </View>

        {/* Tab Cards */}
        <ScrollView
          contentContainerStyle={styles.scrollContent}
          showsVerticalScrollIndicator={false}
        >
          {tabs.map((tab, index) => (
            <TabCard
              key={tab.id}
              tab={tab}
              isActive={tab.id === activeTabId}
              onPress={() => handleTabSelect(tab.id)}
              onClose={() => handleTabClose(tab.id)}
              colors={colors}
              index={index}
            />
          ))}

          {/* Add spacing at bottom */}
          <View style={{ height: 100 }} />
        </ScrollView>

        {/* New Tab Button */}
        <View style={styles.footer}>
          <AnimatedButton onPress={handleNewTab} haptic="success">
            <LinearGradient
              colors={['#667eea', '#764ba2']}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={styles.newTabButton}
            >
              <Ionicons name="add" size={28} color="#fff" />
              <Text style={styles.newTabText}>New Tab</Text>
            </LinearGradient>
          </AnimatedButton>
        </View>
      </Animated.View>
    </Animated.View>
  );
};

interface TabCardProps {
  tab: Tab;
  isActive: boolean;
  onPress: () => void;
  onClose: () => void;
  colors: any;
  index: number;
}

const TabCard: React.FC<TabCardProps> = ({
  tab,
  isActive,
  onPress,
  onClose,
  colors,
  index,
}) => {
  const scaleAnim = useRef(new Animated.Value(0)).current;
  const translateYAnim = useRef(new Animated.Value(50)).current;

  React.useEffect(() => {
    // Stagger entrance animation
    Animated.parallel([
      Animated.spring(scaleAnim, {
        toValue: 1,
        delay: index * 50,
        tension: 300,
        friction: 30,
        useNativeDriver: true,
      }),
      Animated.spring(translateYAnim, {
        toValue: 0,
        delay: index * 50,
        tension: 300,
        friction: 30,
        useNativeDriver: true,
      }),
    ]).start();
  }, []);

  return (
    <Animated.View
      style={[
        styles.cardContainer,
        {
          transform: [{ scale: scaleAnim }, { translateY: translateYAnim }],
        },
      ]}
    >
      <SwipeableRow
        rightActions={[
          {
            icon: 'trash',
            label: 'Close',
            color: '#ef4444',
            onPress: onClose,
          },
        ]}
      >
        <TouchableOpacity onPress={onPress} activeOpacity={0.9}>
          <View
            style={[
              styles.card,
              {
                backgroundColor: colors.card,
                borderColor: isActive ? '#667eea' : colors.border,
              },
            ]}
          >
            {/* Screenshot/Thumbnail */}
            <View style={styles.thumbnail}>
              {tab.screenshot ? (
                <Image
                  source={{ uri: tab.screenshot }}
                  style={styles.thumbnailImage}
                  resizeMode="cover"
                />
              ) : (
                <LinearGradient
                  colors={['#667eea', '#764ba2']}
                  start={{ x: 0, y: 0 }}
                  end={{ x: 1, y: 1 }}
                  style={styles.thumbnailPlaceholder}
                >
                  <Ionicons name="globe-outline" size={48} color="#fff" />
                </LinearGradient>
              )}

              {/* Active indicator */}
              {isActive && (
                <View style={styles.activeIndicator}>
                  <LinearGradient
                    colors={['#667eea', '#764ba2']}
                    start={{ x: 0, y: 0 }}
                    end={{ x: 1, y: 0 }}
                    style={styles.activeBadge}
                  >
                    <Text style={styles.activeText}>Active</Text>
                  </LinearGradient>
                </View>
              )}
            </View>

            {/* Tab Info */}
            <View style={styles.tabInfo}>
              <View style={styles.tabHeader}>
                {tab.favicon && (
                  <Image
                    source={{ uri: tab.favicon }}
                    style={styles.favicon}
                  />
                )}
                <Text
                  style={[styles.tabTitle, { color: colors.text }]}
                  numberOfLines={1}
                >
                  {tab.title || 'New Tab'}
                </Text>
              </View>
              <Text
                style={[styles.tabUrl, { color: colors.textSecondary }]}
                numberOfLines={1}
              >
                {tab.url || 'about:blank'}
              </Text>
            </View>

            {/* Close button */}
            <TouchableOpacity
              onPress={onClose}
              style={styles.closeButton}
              hitSlop={{ top: 10, right: 10, bottom: 10, left: 10 }}
            >
              <Ionicons name="close-circle" size={24} color={colors.textSecondary} />
            </TouchableOpacity>
          </View>
        </TouchableOpacity>
      </SwipeableRow>
    </Animated.View>
  );
};

const styles = StyleSheet.create({
  overlay: {
    position: 'absolute',
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    zIndex: 1000,
  },
  container: {
    flex: 1,
    paddingTop: 60,
  },
  header: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    paddingHorizontal: 24,
    marginBottom: 24,
  },
  headerTitle: {
    fontSize: 32,
    fontWeight: '700',
  },
  scrollContent: {
    paddingHorizontal: 24,
    gap: 16,
  },
  cardContainer: {
    marginBottom: 16,
  },
  card: {
    width: CARD_WIDTH,
    borderRadius: 20,
    borderWidth: 2,
    overflow: 'hidden',
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.1,
    shadowRadius: 12,
    elevation: 5,
  },
  thumbnail: {
    width: '100%',
    height: 280,
    position: 'relative',
  },
  thumbnailImage: {
    width: '100%',
    height: '100%',
  },
  thumbnailPlaceholder: {
    width: '100%',
    height: '100%',
    justifyContent: 'center',
    alignItems: 'center',
  },
  activeIndicator: {
    position: 'absolute',
    top: 12,
    right: 12,
  },
  activeBadge: {
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderRadius: 12,
  },
  activeText: {
    color: '#fff',
    fontSize: 12,
    fontWeight: '600',
  },
  tabInfo: {
    padding: 16,
    gap: 8,
  },
  tabHeader: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
  },
  favicon: {
    width: 20,
    height: 20,
    borderRadius: 4,
  },
  tabTitle: {
    fontSize: 16,
    fontWeight: '600',
    flex: 1,
  },
  tabUrl: {
    fontSize: 14,
  },
  closeButton: {
    position: 'absolute',
    top: 12,
    left: 12,
  },
  footer: {
    position: 'absolute',
    bottom: 0,
    left: 0,
    right: 0,
    padding: 24,
    paddingBottom: 40,
  },
  newTabButton: {
    flexDirection: 'row',
    justifyContent: 'center',
    alignItems: 'center',
    paddingVertical: 16,
    borderRadius: 16,
    gap: 8,
  },
  newTabText: {
    color: '#fff',
    fontSize: 18,
    fontWeight: '600',
  },
});
