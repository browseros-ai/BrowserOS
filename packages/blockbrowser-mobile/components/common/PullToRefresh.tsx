/**
 * PullToRefresh - Custom pull-to-refresh with brand colors
 */

import React, { useRef, useEffect } from 'react';
import {
  View,
  Animated,
  PanResponder,
  StyleSheet,
  ActivityIndicator,
  useColorScheme,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { LinearGradient } from 'expo-linear-gradient';
import { HapticFeedback } from '@/lib/utils/haptics';
import { Colors } from '@/constants/Colors';

interface PullToRefreshProps {
  onRefresh: () => Promise<void>;
  children: React.ReactNode;
  refreshing?: boolean;
}

const PULL_THRESHOLD = 80;
const MAX_PULL = 120;

export const PullToRefresh: React.FC<PullToRefreshProps> = ({
  onRefresh,
  children,
  refreshing = false,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;

  const pullDistance = useRef(new Animated.Value(0)).current;
  const rotation = useRef(new Animated.Value(0)).current;
  const isRefreshing = useRef(false);
  const hasTriggeredHaptic = useRef(false);

  useEffect(() => {
    if (refreshing) {
      // Animate rotation while refreshing
      Animated.loop(
        Animated.timing(rotation, {
          toValue: 1,
          duration: 1000,
          useNativeDriver: true,
        })
      ).start();
    } else {
      rotation.setValue(0);
    }
  }, [refreshing]);

  const panResponder = useRef(
    PanResponder.create({
      onMoveShouldSetPanResponder: (_, gestureState) => {
        // Only respond to downward pull at the top
        return gestureState.dy > 10 && !isRefreshing.current;
      },

      onPanResponderGrant: () => {
        hasTriggeredHaptic.current = false;
      },

      onPanResponderMove: (_, gestureState) => {
        if (gestureState.dy < 0 || isRefreshing.current) return;

        // Apply resistance to pull
        const resistance = gestureState.dy / 2;
        const clampedValue = Math.min(resistance, MAX_PULL);

        pullDistance.setValue(clampedValue);

        // Haptic at threshold
        if (clampedValue >= PULL_THRESHOLD && !hasTriggeredHaptic.current) {
          HapticFeedback.medium();
          hasTriggeredHaptic.current = true;
        }
      },

      onPanResponderRelease: async (_, gestureState) => {
        const pullValue = Math.min(gestureState.dy / 2, MAX_PULL);

        if (pullValue >= PULL_THRESHOLD && !isRefreshing.current) {
          // Trigger refresh
          isRefreshing.current = true;
          HapticFeedback.success();

          // Animate to loading position
          Animated.spring(pullDistance, {
            toValue: PULL_THRESHOLD,
            useNativeDriver: false,
            tension: 300,
            friction: 30,
          }).start();

          try {
            await onRefresh();
          } finally {
            isRefreshing.current = false;
            // Reset
            Animated.spring(pullDistance, {
              toValue: 0,
              useNativeDriver: false,
              tension: 300,
              friction: 30,
            }).start();
          }
        } else {
          // Reset without refresh
          HapticFeedback.light();
          Animated.spring(pullDistance, {
            toValue: 0,
            useNativeDriver: false,
            tension: 300,
            friction: 30,
          }).start();
        }
      },
    })
  ).current;

  const indicatorOpacity = pullDistance.interpolate({
    inputRange: [0, PULL_THRESHOLD],
    outputRange: [0, 1],
    extrapolate: 'clamp',
  });

  const indicatorScale = pullDistance.interpolate({
    inputRange: [0, PULL_THRESHOLD],
    outputRange: [0.5, 1],
    extrapolate: 'clamp',
  });

  const rotationDegrees = rotation.interpolate({
    inputRange: [0, 1],
    outputRange: ['0deg', '360deg'],
  });

  return (
    <View style={styles.container} {...panResponder.panHandlers}>
      {/* Pull indicator */}
      <Animated.View
        style={[
          styles.indicator,
          {
            height: pullDistance,
            opacity: indicatorOpacity,
          },
        ]}
      >
        <Animated.View
          style={{
            transform: [{ scale: indicatorScale }, { rotate: rotationDegrees }],
          }}
        >
          {refreshing || isRefreshing.current ? (
            <ActivityIndicator size="small" color="#667eea" />
          ) : (
            <LinearGradient
              colors={['#667eea', '#764ba2']}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 1 }}
              style={styles.iconContainer}
            >
              <Ionicons name="arrow-down" size={20} color="#fff" />
            </LinearGradient>
          )}
        </Animated.View>
      </Animated.View>

      {/* Content */}
      <Animated.View
        style={[
          styles.content,
          {
            transform: [{ translateY: pullDistance }],
          },
        ]}
      >
        {children}
      </Animated.View>
    </View>
  );
};

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  indicator: {
    position: 'absolute',
    top: 0,
    left: 0,
    right: 0,
    justifyContent: 'center',
    alignItems: 'center',
    zIndex: 10,
  },
  iconContainer: {
    width: 36,
    height: 36,
    borderRadius: 18,
    justifyContent: 'center',
    alignItems: 'center',
  },
  content: {
    flex: 1,
  },
});
