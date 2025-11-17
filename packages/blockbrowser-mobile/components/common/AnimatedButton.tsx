/**
 * AnimatedButton - Premium button with smooth animations and haptics
 */

import React, { useRef } from 'react';
import {
  TouchableOpacity,
  Animated,
  StyleSheet,
  ViewStyle,
  StyleProp,
} from 'react-native';
import { HapticFeedback } from '@/lib/utils/haptics';

interface AnimatedButtonProps {
  onPress: () => void;
  style?: StyleProp<ViewStyle>;
  children: React.ReactNode;
  haptic?: 'light' | 'medium' | 'heavy' | 'success';
  scaleEffect?: boolean;
  disabled?: boolean;
}

export const AnimatedButton: React.FC<AnimatedButtonProps> = ({
  onPress,
  style,
  children,
  haptic = 'light',
  scaleEffect = true,
  disabled = false,
}) => {
  const scaleAnim = useRef(new Animated.Value(1)).current;

  const handlePressIn = () => {
    if (scaleEffect && !disabled) {
      Animated.spring(scaleAnim, {
        toValue: 0.95,
        useNativeDriver: true,
        tension: 300,
        friction: 10,
      }).start();
    }
  };

  const handlePressOut = () => {
    if (scaleEffect && !disabled) {
      Animated.spring(scaleAnim, {
        toValue: 1,
        useNativeDriver: true,
        tension: 300,
        friction: 10,
      }).start();
    }
  };

  const handlePress = () => {
    if (disabled) return;

    // Haptic feedback
    switch (haptic) {
      case 'light':
        HapticFeedback.light();
        break;
      case 'medium':
        HapticFeedback.medium();
        break;
      case 'heavy':
        HapticFeedback.heavy();
        break;
      case 'success':
        HapticFeedback.success();
        break;
    }

    onPress();
  };

  return (
    <TouchableOpacity
      activeOpacity={0.8}
      onPressIn={handlePressIn}
      onPressOut={handlePressOut}
      onPress={handlePress}
      disabled={disabled}
    >
      <Animated.View
        style={[
          style,
          {
            transform: [{ scale: scaleAnim }],
            opacity: disabled ? 0.5 : 1,
          },
        ]}
      >
        {children}
      </Animated.View>
    </TouchableOpacity>
  );
};
