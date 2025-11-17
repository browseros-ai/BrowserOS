import React from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  StyleSheet,
  Modal,
  useColorScheme,
  Platform,
  Alert,
} from 'react-native';
import { LinearGradient } from 'expo-linear-gradient';
import { Ionicons } from '@expo/vector-icons';
import { LiquidGlassView } from '@callstack/liquid-glass';
import { isLiquidGlassSupported } from '@/lib/utils/liquid-glass-helper';
import { Colors } from '@/constants/Colors';
import { useBrowserStore } from '@/store/browser-store';

interface BrowserMenuProps {
  visible: boolean;
  onClose: () => void;
  onNewTab: () => void;
  onNewIncognitoTab: () => void;
  onViewHistory: () => void;
  onViewBookmarks: () => void;
  onSettings: () => void;
}

export const BrowserMenu: React.FC<BrowserMenuProps> = ({
  visible,
  onClose,
  onNewTab,
  onNewIncognitoTab,
  onViewHistory,
  onViewBookmarks,
  onSettings,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;
  const useLiquidGlass = Platform.OS === 'ios' && isLiquidGlassSupported();
  const { clearHistory, clearBookmarks } = useBrowserStore();

  const menuItems = [
    {
      icon: 'add-circle-outline',
      title: 'New Tab',
      onPress: () => {
        onNewTab();
        onClose();
      },
    },
    {
      icon: 'eye-off-outline',
      title: 'New Incognito Tab',
      onPress: () => {
        onNewIncognitoTab();
        onClose();
      },
    },
    {
      icon: 'time-outline',
      title: 'History',
      onPress: () => {
        onViewHistory();
        onClose();
      },
    },
    {
      icon: 'bookmark-outline',
      title: 'Bookmarks',
      onPress: () => {
        onViewBookmarks();
        onClose();
      },
    },
    {
      icon: 'trash-outline',
      title: 'Clear History',
      color: 'error' as const,
      onPress: () => {
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
                onClose();
              },
            },
          ]
        );
      },
    },
    {
      icon: 'settings-outline',
      title: 'Settings',
      onPress: () => {
        onSettings();
        onClose();
      },
    },
  ];

  const MenuContent = () => (
    <View
      style={[
        styles.menuContent,
        {
          backgroundColor: colors.background,
        },
      ]}
    >
      {/* Header with Gradient */}
      <LinearGradient
        colors={['#667eea', '#764ba2']}
        start={{ x: 0, y: 0 }}
        end={{ x: 1, y: 1 }}
        style={styles.gradientHeader}
      >
        <View style={styles.headerContent}>
          <View style={styles.headerLeft}>
            <View style={styles.menuLogoPlaceholder}>
              <Text style={styles.menuLogoText}>B</Text>
            </View>
            <Text style={styles.menuTitle}>Browser Menu</Text>
          </View>
          <TouchableOpacity onPress={onClose} style={styles.closeButton}>
            <Ionicons name="close" size={28} color="#fff" />
          </TouchableOpacity>
        </View>
      </LinearGradient>

      {/* Menu Items */}
      <View style={styles.menuItems}>
        {menuItems.map((item, index) => (
          <TouchableOpacity
            key={index}
            style={[
              styles.menuItem,
              {
                backgroundColor: colors.surface,
                borderColor: colors.border,
              },
            ]}
            onPress={item.onPress}
            activeOpacity={0.7}
          >
            <View
              style={[
                styles.iconContainer,
                {
                  backgroundColor:
                    item.color === 'error'
                      ? colors.error + '20'
                      : colors.primary + '20',
                },
              ]}
            >
              <Ionicons
                name={item.icon as any}
                size={24}
                color={item.color === 'error' ? colors.error : colors.primary}
              />
            </View>
            <Text
              style={[
                styles.menuItemText,
                {
                  color: item.color === 'error' ? colors.error : colors.text,
                },
              ]}
            >
              {item.title}
            </Text>
            <Ionicons
              name="chevron-forward"
              size={20}
              color={colors.textSecondary}
            />
          </TouchableOpacity>
        ))}
      </View>
    </View>
  );

  return (
    <Modal
      visible={visible}
      animationType="slide"
      presentationStyle="pageSheet"
      onRequestClose={onClose}
    >
      {useLiquidGlass ? (
        <LiquidGlassView
          style={styles.modalGlass}
          effect="regular"
          tintColor={colorScheme === 'dark' ? '#000000' : '#f2f2f7'}
          colorScheme={colorScheme || 'light'}
        >
          <MenuContent />
        </LiquidGlassView>
      ) : (
        <MenuContent />
      )}
    </Modal>
  );
};

const styles = StyleSheet.create({
  modalGlass: {
    flex: 1,
  },
  menuContent: {
    flex: 1,
  },
  gradientHeader: {
    paddingTop: Platform.OS === 'ios' ? 60 : 24,
    paddingBottom: 20,
    paddingHorizontal: 20,
    borderBottomLeftRadius: 24,
    borderBottomRightRadius: 24,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.3,
    shadowRadius: 8,
    elevation: 8,
  },
  headerContent: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
  },
  headerLeft: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 12,
  },
  menuLogoPlaceholder: {
    width: 44,
    height: 44,
    borderRadius: 12,
    backgroundColor: 'rgba(255, 255, 255, 0.2)',
    alignItems: 'center',
    justifyContent: 'center',
    borderWidth: 2,
    borderColor: 'rgba(255, 255, 255, 0.3)',
  },
  menuLogoText: {
    fontSize: 24,
    fontWeight: '900',
    color: '#fff',
  },
  menuTitle: {
    fontSize: 24,
    fontWeight: '800',
    color: '#fff',
    letterSpacing: 0.5,
  },
  closeButton: {
    padding: 4,
  },
  menuItems: {
    padding: 16,
    paddingTop: 20,
    gap: 12,
  },
  menuItem: {
    flexDirection: 'row',
    alignItems: 'center',
    padding: 16,
    borderRadius: 12,
    borderWidth: 1,
    gap: 12,
  },
  iconContainer: {
    width: 40,
    height: 40,
    borderRadius: 20,
    alignItems: 'center',
    justifyContent: 'center',
  },
  menuItemText: {
    flex: 1,
    fontSize: 16,
    fontWeight: '500',
  },
});
