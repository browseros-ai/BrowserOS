/**
 * LoadingSkeleton - Beautiful loading skeletons (no blank screens!)
 */

import React, { useEffect, useRef } from 'react';
import {
  View,
  Animated,
  StyleSheet,
  useColorScheme,
  ViewStyle,
  StyleProp,
} from 'react-native';
import { LinearGradient } from 'expo-linear-gradient';
import { Colors } from '@/constants/Colors';

interface SkeletonProps {
  width?: number | string;
  height?: number;
  borderRadius?: number;
  style?: StyleProp<ViewStyle>;
}

export const Skeleton: React.FC<SkeletonProps> = ({
  width = '100%',
  height = 20,
  borderRadius = 8,
  style,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;
  const shimmerAnim = useRef(new Animated.Value(0)).current;

  useEffect(() => {
    Animated.loop(
      Animated.sequence([
        Animated.timing(shimmerAnim, {
          toValue: 1,
          duration: 1000,
          useNativeDriver: true,
        }),
        Animated.timing(shimmerAnim, {
          toValue: 0,
          duration: 1000,
          useNativeDriver: true,
        }),
      ])
    ).start();
  }, []);

  const opacity = shimmerAnim.interpolate({
    inputRange: [0, 1],
    outputRange: [0.3, 0.7],
  });

  return (
    <Animated.View
      style={[
        {
          width,
          height,
          borderRadius,
          backgroundColor: colors.border,
          opacity,
        },
        style,
      ]}
    />
  );
};

/**
 * Page Loading Skeleton
 */
export const PageSkeleton: React.FC = () => {
  return (
    <View style={styles.pageContainer}>
      {/* Header */}
      <View style={styles.header}>
        <Skeleton width={150} height={24} />
        <Skeleton width={100} height={16} style={{ marginTop: 8 }} />
      </View>

      {/* Content blocks */}
      <View style={styles.content}>
        <Skeleton width="100%" height={200} borderRadius={12} />

        <View style={styles.textBlock}>
          <Skeleton width="100%" height={16} />
          <Skeleton width="95%" height={16} />
          <Skeleton width="90%" height={16} />
        </View>

        <View style={styles.textBlock}>
          <Skeleton width="100%" height={16} />
          <Skeleton width="98%" height={16} />
          <Skeleton width="85%" height={16} />
          <Skeleton width="92%" height={16} />
        </View>
      </View>
    </View>
  );
};

/**
 * Tab List Skeleton
 */
export const TabListSkeleton: React.FC = () => {
  return (
    <View style={styles.tabListContainer}>
      {[1, 2, 3].map((i) => (
        <View key={i} style={styles.tabCard}>
          <View style={styles.tabHeader}>
            <Skeleton width={24} height={24} borderRadius={12} />
            <Skeleton width={150} height={16} style={{ marginLeft: 12 }} />
          </View>
          <Skeleton width="100%" height={12} style={{ marginTop: 8 }} />
          <Skeleton width="80%" height={12} style={{ marginTop: 4 }} />
        </View>
      ))}
    </View>
  );
};

/**
 * Chat Message Skeleton
 */
export const ChatSkeleton: React.FC = () => {
  return (
    <View style={styles.chatContainer}>
      {/* User message */}
      <View style={[styles.messageBubble, styles.userBubble]}>
        <Skeleton width="80%" height={16} />
        <Skeleton width="60%" height={16} style={{ marginTop: 4 }} />
      </View>

      {/* AI message */}
      <View style={[styles.messageBubble, styles.aiBubble]}>
        <Skeleton width="90%" height={16} />
        <Skeleton width="95%" height={16} style={{ marginTop: 4 }} />
        <Skeleton width="75%" height={16} style={{ marginTop: 4 }} />
      </View>
    </View>
  );
};

const styles = StyleSheet.create({
  pageContainer: {
    padding: 16,
  },
  header: {
    marginBottom: 24,
  },
  content: {
    gap: 16,
  },
  textBlock: {
    gap: 8,
  },
  tabListContainer: {
    padding: 16,
    gap: 12,
  },
  tabCard: {
    padding: 16,
    borderRadius: 12,
  },
  tabHeader: {
    flexDirection: 'row',
    alignItems: 'center',
  },
  chatContainer: {
    padding: 16,
    gap: 16,
  },
  messageBubble: {
    padding: 12,
    borderRadius: 16,
    maxWidth: '80%',
  },
  userBubble: {
    alignSelf: 'flex-end',
  },
  aiBubble: {
    alignSelf: 'flex-start',
  },
});
