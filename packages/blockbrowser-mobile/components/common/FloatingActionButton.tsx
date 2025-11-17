/**
 * FloatingActionButton - Material Design FAB with gradient
 */

import React, { useRef } from 'react';
import {
  TouchableOpacity,
  Animated,
  StyleSheet,
  ViewStyle,
  StyleProp,
  Platform,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { LinearGradient } from 'expo-linear-gradient';
import { HapticFeedback } from '@/lib/utils/haptics';

interface FloatingActionButtonProps {
  icon: string;
  onPress: () => void;
  style?: StyleProp<ViewStyle>;
  size?: 'small' | 'medium' | 'large';
  position?: 'bottom-right' | 'bottom-center' | 'bottom-left';
}

export const FloatingActionButton: React.FC<FloatingActionButtonProps> = ({
  icon,
  onPress,
  style,
  size = 'medium',
  position = 'bottom-right',
}) => {
  const scaleAnim = useRef(new Animated.Value(1)).current;
  const rotateAnim = useRef(new Animated.Value(0)).current;

  const handlePressIn = () => {
    Animated.spring(scaleAnim, {
      toValue: 0.9,
      tension: 300,
      friction: 10,
      useNativeDriver: true,
    }).start();
  };

  const handlePressOut = () => {
    Animated.spring(scaleAnim, {
      toValue: 1,
      tension: 300,
      friction: 10,
      useNativeDriver: true,
    }).start();
  };

  const handlePress = () => {
    HapticFeedback.medium();

    // Rotate animation
    Animated.sequence([
      Animated.timing(rotateAnim, {
        toValue: 1,
        duration: 200,
        useNativeDriver: true,
      }),
      Animated.timing(rotateAnim, {
        toValue: 0,
        duration: 0,
        useNativeDriver: true,
      }),
    ]).start();

    onPress();
  };

  const getSizeStyle = () => {
    switch (size) {
      case 'small':
        return { width: 48, height: 48, iconSize: 20 };
      case 'large':
        return { width: 72, height: 72, iconSize: 32 };
      case 'medium':
      default:
        return { width: 60, height: 60, iconSize: 28 };
    }
  };

  const getPositionStyle = (): ViewStyle => {
    const base = {
      position: 'absolute' as const,
      bottom: 24,
    };

    switch (position) {
      case 'bottom-left':
        return { ...base, left: 24 };
      case 'bottom-center':
        return { ...base, left: '50%', marginLeft: -30 };
      case 'bottom-right':
      default:
        return { ...base, right: 24 };
    }
  };

  const sizeStyle = getSizeStyle();
  const rotation = rotateAnim.interpolate({
    inputRange: [0, 1],
    outputRange: ['0deg', '360deg'],
  });

  return (
    <TouchableOpacity
      activeOpacity={1}
      onPressIn={handlePressIn}
      onPressOut={handlePressOut}
      onPress={handlePress}
      style={[getPositionStyle(), style]}
    >
      <Animated.View
        style={[
          {
            transform: [{ scale: scaleAnim }, { rotate: rotation }],
          },
        ]}
      >
        <LinearGradient
          colors={['#667eea', '#764ba2']}
          start={{ x: 0, y: 0 }}
          end={{ x: 1, y: 1 }}
          style={[
            styles.fab,
            {
              width: sizeStyle.width,
              height: sizeStyle.height,
              borderRadius: sizeStyle.width / 2,
            },
          ]}
        >
          <Ionicons name={icon as any} size={sizeStyle.iconSize} color="#fff" />
        </LinearGradient>
      </Animated.View>
    </TouchableOpacity>
  );
};

const styles = StyleSheet.create({
  fab: {
    justifyContent: 'center',
    alignItems: 'center',
    ...Platform.select({
      ios: {
        shadowColor: '#000',
        shadowOffset: { width: 0, height: 4 },
        shadowOpacity: 0.3,
        shadowRadius: 12,
      },
      android: {
        elevation: 8,
      },
    }),
  },
});
