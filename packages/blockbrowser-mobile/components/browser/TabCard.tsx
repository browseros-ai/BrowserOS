import React from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  Image,
  StyleSheet,
  Platform,
  useColorScheme,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { LiquidGlassView } from '@callstack/liquid-glass';
import { isLiquidGlassSupported } from '@/lib/utils/liquid-glass-helper';
import { Tab } from '@/store/browser-store';
import { Colors } from '@/constants/Colors';
import { URLValidator } from '@/lib/utils/url-validator';

interface TabCardProps {
  tab: Tab;
  isActive: boolean;
  onSelect: () => void;
  onClose: () => void;
}

export const TabCard: React.FC<TabCardProps> = ({
  tab,
  isActive,
  onSelect,
  onClose,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;
  const useLiquidGlass = Platform.OS === 'ios' && isLiquidGlassSupported();

  const faviconUrl = tab.favicon || URLValidator.getFaviconURL(tab.url);

  const CardContent = () => (
    <TouchableOpacity
      style={[
        styles.card,
        {
          backgroundColor: colors.surface,
          borderColor: isActive ? colors.primary : colors.border,
        },
        isActive && styles.activeCard,
      ]}
      onPress={onSelect}
      activeOpacity={0.8}
    >
      {/* Card Header */}
      <View style={styles.header}>
        {/* Favicon */}
        <Image
          source={{ uri: faviconUrl }}
          style={styles.favicon}
          defaultSource={require('@/assets/favicon-placeholder.png')}
        />

        {/* Title */}
        <Text
          style={[styles.title, { color: colors.text }]}
          numberOfLines={1}
          ellipsizeMode="tail"
        >
          {tab.title || 'New Tab'}
        </Text>

        {/* Close Button */}
        <TouchableOpacity
          onPress={(e) => {
            e.stopPropagation();
            onClose();
          }}
          style={styles.closeButton}
          hitSlop={{ top: 10, bottom: 10, left: 10, right: 10 }}
        >
          <Ionicons name="close" size={20} color={colors.error} />
        </TouchableOpacity>
      </View>

      {/* URL */}
      <Text
        style={[styles.url, { color: colors.textSecondary }]}
        numberOfLines={1}
        ellipsizeMode="tail"
      >
        {URLValidator.shortenURL(tab.url, 60)}
      </Text>

      {/* Active Indicator */}
      {isActive && (
        <View style={styles.activeIndicator}>
          <Ionicons name="checkmark-circle" size={18} color={colors.success} />
          <Text style={[styles.activeText, { color: colors.success }]}>
            Active
          </Text>
        </View>
      )}

      {/* Loading Indicator */}
      {tab.isLoading && (
        <View style={styles.loadingIndicator}>
          <View
            style={[
              styles.progressBar,
              {
                width: `${tab.progress * 100}%`,
                backgroundColor: colors.primary,
              },
            ]}
          />
        </View>
      )}
    </TouchableOpacity>
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

const styles = StyleSheet.create({
  glassCard: {
    borderRadius: 16,
    overflow: 'hidden',
    marginBottom: 12,
  },
  card: {
    borderRadius: 16,
    padding: 16,
    borderWidth: 2,
    borderColor: 'transparent',
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.1,
    shadowRadius: 8,
    elevation: 3,
  },
  activeCard: {
    borderWidth: 2,
  },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    marginBottom: 8,
  },
  favicon: {
    width: 20,
    height: 20,
    marginRight: 8,
    borderRadius: 4,
  },
  title: {
    flex: 1,
    fontSize: 16,
    fontWeight: '600',
  },
  closeButton: {
    padding: 4,
  },
  url: {
    fontSize: 13,
    marginTop: 4,
  },
  activeIndicator: {
    flexDirection: 'row',
    alignItems: 'center',
    marginTop: 8,
    gap: 4,
  },
  activeText: {
    fontSize: 12,
    fontWeight: '500',
  },
  loadingIndicator: {
    marginTop: 8,
    height: 2,
    backgroundColor: 'rgba(0,0,0,0.1)',
    borderRadius: 1,
    overflow: 'hidden',
  },
  progressBar: {
    height: '100%',
    borderRadius: 1,
  },
});
