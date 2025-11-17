/**
 * ContextMenu - Long-press context menu with smooth animations
 */

import React, { useRef, useEffect } from 'react';
import {
  View,
  Text,
  Modal,
  TouchableOpacity,
  Animated,
  StyleSheet,
  useColorScheme,
  Dimensions,
  TouchableWithoutFeedback,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { Colors } from '@/constants/Colors';
import { HapticFeedback } from '@/lib/utils/haptics';

const SCREEN_HEIGHT = Dimensions.get('window').height;

export interface ContextMenuItem {
  icon: string;
  label: string;
  onPress: () => void;
  destructive?: boolean;
  disabled?: boolean;
}

interface ContextMenuProps {
  visible: boolean;
  onClose: () => void;
  items: ContextMenuItem[];
  title?: string;
  position?: { x: number; y: number };
}

export const ContextMenu: React.FC<ContextMenuProps> = ({
  visible,
  onClose,
  items,
  title,
  position,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;

  const opacity = useRef(new Animated.Value(0)).current;
  const scale = useRef(new Animated.Value(0.8)).current;
  const translateY = useRef(new Animated.Value(20)).current;

  useEffect(() => {
    if (visible) {
      HapticFeedback.medium();
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
        Animated.spring(translateY, {
          toValue: 0,
          tension: 300,
          friction: 30,
          useNativeDriver: true,
        }),
      ]).start();
    } else {
      Animated.parallel([
        Animated.timing(opacity, {
          toValue: 0,
          duration: 150,
          useNativeDriver: true,
        }),
        Animated.timing(scale, {
          toValue: 0.8,
          duration: 150,
          useNativeDriver: true,
        }),
      ]).start();
    }
  }, [visible]);

  const handleItemPress = (item: ContextMenuItem) => {
    if (item.disabled) return;

    HapticFeedback.light();
    onClose();
    setTimeout(() => {
      item.onPress();
    }, 150);
  };

  return (
    <Modal
      transparent
      visible={visible}
      animationType="none"
      onRequestClose={onClose}
    >
      <TouchableWithoutFeedback onPress={onClose}>
        <View style={styles.overlay}>
          <Animated.View
            style={[
              styles.backdrop,
              {
                opacity: opacity.interpolate({
                  inputRange: [0, 1],
                  outputRange: [0, 0.5],
                }),
              },
            ]}
          />

          <TouchableWithoutFeedback>
            <Animated.View
              style={[
                styles.menu,
                {
                  backgroundColor: colors.card,
                  opacity,
                  transform: [{ scale }, { translateY }],
                },
                position && {
                  top: position.y,
                  left: position.x,
                },
              ]}
            >
              {title && (
                <View style={styles.header}>
                  <Text style={[styles.title, { color: colors.textSecondary }]}>
                    {title}
                  </Text>
                </View>
              )}

              {items.map((item, index) => (
                <React.Fragment key={index}>
                  <TouchableOpacity
                    onPress={() => handleItemPress(item)}
                    disabled={item.disabled}
                    style={[
                      styles.menuItem,
                      item.disabled && styles.menuItemDisabled,
                    ]}
                    activeOpacity={0.7}
                  >
                    <Ionicons
                      name={item.icon as any}
                      size={22}
                      color={
                        item.disabled
                          ? colors.textSecondary
                          : item.destructive
                          ? '#ef4444'
                          : colors.text
                      }
                    />
                    <Text
                      style={[
                        styles.menuItemLabel,
                        {
                          color: item.disabled
                            ? colors.textSecondary
                            : item.destructive
                            ? '#ef4444'
                            : colors.text,
                        },
                      ]}
                    >
                      {item.label}
                    </Text>
                  </TouchableOpacity>

                  {index < items.length - 1 && (
                    <View
                      style={[styles.separator, { backgroundColor: colors.border }]}
                    />
                  )}
                </React.Fragment>
              ))}
            </Animated.View>
          </TouchableWithoutFeedback>
        </View>
      </TouchableWithoutFeedback>
    </Modal>
  );
};

const styles = StyleSheet.create({
  overlay: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
  },
  backdrop: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: '#000',
  },
  menu: {
    minWidth: 250,
    maxWidth: 300,
    borderRadius: 16,
    paddingVertical: 8,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 8 },
    shadowOpacity: 0.2,
    shadowRadius: 24,
    elevation: 8,
  },
  header: {
    paddingHorizontal: 16,
    paddingVertical: 12,
    borderBottomWidth: 1,
    borderBottomColor: 'rgba(0,0,0,0.1)',
  },
  title: {
    fontSize: 14,
    fontWeight: '600',
  },
  menuItem: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 14,
    gap: 12,
  },
  menuItemDisabled: {
    opacity: 0.5,
  },
  menuItemLabel: {
    fontSize: 16,
    fontWeight: '500',
  },
  separator: {
    height: 1,
    marginHorizontal: 16,
  },
});
