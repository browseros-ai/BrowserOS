/**
 * SwipeableRow - iOS-style swipeable row for deleting/actions
 */

import React, { useRef } from 'react';
import {
  View,
  Animated,
  PanResponder,
  StyleSheet,
  TouchableOpacity,
  Text,
  useColorScheme,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { HapticFeedback } from '@/lib/utils/haptics';
import { Colors } from '@/constants/Colors';

interface SwipeAction {
  icon: string;
  label: string;
  color: string;
  onPress: () => void;
}

interface SwipeableRowProps {
  children: React.ReactNode;
  leftActions?: SwipeAction[];
  rightActions?: SwipeAction[];
  onSwipeStart?: () => void;
  onSwipeEnd?: () => void;
}

export const SwipeableRow: React.FC<SwipeableRowProps> = ({
  children,
  leftActions = [],
  rightActions = [],
  onSwipeStart,
  onSwipeEnd,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;

  const translateX = useRef(new Animated.Value(0)).current;
  const lastOffset = useRef(0);

  const panResponder = useRef(
    PanResponder.create({
      onMoveShouldSetPanResponder: (_, gestureState) => {
        // Only respond to horizontal swipes
        return Math.abs(gestureState.dx) > 10;
      },
      onPanResponderGrant: () => {
        onSwipeStart?.();
        HapticFeedback.selection();
      },
      onPanResponderMove: (_, gestureState) => {
        const maxLeft = leftActions.length * 80;
        const maxRight = rightActions.length * -80;

        const newValue = lastOffset.current + gestureState.dx;
        const clampedValue = Math.max(maxRight, Math.min(maxLeft, newValue));

        translateX.setValue(clampedValue);
      },
      onPanResponderRelease: (_, gestureState) => {
        const threshold = 80;
        let finalPosition = 0;

        if (gestureState.dx > threshold && leftActions.length > 0) {
          finalPosition = leftActions.length * 80;
          HapticFeedback.medium();
        } else if (gestureState.dx < -threshold && rightActions.length > 0) {
          finalPosition = rightActions.length * -80;
          HapticFeedback.medium();
        } else {
          HapticFeedback.light();
        }

        lastOffset.current = finalPosition;

        Animated.spring(translateX, {
          toValue: finalPosition,
          useNativeDriver: true,
          tension: 300,
          friction: 30,
        }).start();

        onSwipeEnd?.();
      },
    })
  ).current;

  const handleActionPress = (action: SwipeAction) => {
    HapticFeedback.heavy();

    // Reset position
    Animated.spring(translateX, {
      toValue: 0,
      useNativeDriver: true,
      tension: 300,
      friction: 30,
    }).start();

    lastOffset.current = 0;
    action.onPress();
  };

  return (
    <View style={styles.container}>
      {/* Left Actions */}
      {leftActions.length > 0 && (
        <View style={styles.actionsContainer}>
          {leftActions.map((action, index) => (
            <TouchableOpacity
              key={index}
              style={[styles.action, { backgroundColor: action.color }]}
              onPress={() => handleActionPress(action)}
            >
              <Ionicons name={action.icon as any} size={24} color="#fff" />
              <Text style={styles.actionLabel}>{action.label}</Text>
            </TouchableOpacity>
          ))}
        </View>
      )}

      {/* Right Actions */}
      {rightActions.length > 0 && (
        <View style={[styles.actionsContainer, styles.rightActions]}>
          {rightActions.map((action, index) => (
            <TouchableOpacity
              key={index}
              style={[styles.action, { backgroundColor: action.color }]}
              onPress={() => handleActionPress(action)}
            >
              <Ionicons name={action.icon as any} size={24} color="#fff" />
              <Text style={styles.actionLabel}>{action.label}</Text>
            </TouchableOpacity>
          ))}
        </View>
      )}

      {/* Content */}
      <Animated.View
        style={[
          styles.content,
          { transform: [{ translateX }] },
        ]}
        {...panResponder.panHandlers}
      >
        {children}
      </Animated.View>
    </View>
  );
};

const styles = StyleSheet.create({
  container: {
    position: 'relative',
    overflow: 'hidden',
  },
  actionsContainer: {
    position: 'absolute',
    top: 0,
    left: 0,
    bottom: 0,
    flexDirection: 'row',
  },
  rightActions: {
    left: undefined,
    right: 0,
  },
  action: {
    width: 80,
    justifyContent: 'center',
    alignItems: 'center',
    gap: 4,
  },
  actionLabel: {
    color: '#fff',
    fontSize: 12,
    fontWeight: '600',
  },
  content: {
    backgroundColor: 'transparent',
  },
});
