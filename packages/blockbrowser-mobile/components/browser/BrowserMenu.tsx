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
import { Ionicons } from '@expo/vector-icons';
import { LiquidGlassView, isLiquidGlassSupported } from '@callstack/liquid-glass';
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
      {/* Header */}
      <View
        style={[
          styles.header,
          {
            borderBottomColor: colors.border,
          },
        ]}
      >
        <Text style={[styles.title, { color: colors.text }]}>Browser Menu</Text>
        <TouchableOpacity onPress={onClose}>
          <Ionicons name="close" size={28} color={colors.textSecondary} />
        </TouchableOpacity>
      </View>

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
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    padding: 16,
    paddingTop: Platform.OS === 'ios' ? 60 : 16,
    borderBottomWidth: 1,
  },
  title: {
    fontSize: 24,
    fontWeight: '700',
  },
  menuItems: {
    padding: 16,
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
