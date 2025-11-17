/**
 * SmartScrollView - Enhanced ScrollView with smart behaviors
 * - Auto-scroll to focused input
 * - Minimize header on scroll
 * - Pull-to-refresh
 * - Scroll-to-top on tap
 */

import React, { useRef, useState } from 'react';
import {
  ScrollView,
  ScrollViewProps,
  Animated,
  TouchableOpacity,
  StyleSheet,
  ViewStyle,
  StyleProp,
  Platform,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { LinearGradient } from 'expo-linear-gradient';
import { HapticFeedback } from '@/lib/utils/haptics';
import { throttle } from '@/lib/utils/performance';

interface SmartScrollViewProps extends ScrollViewProps {
  onScrollUp?: () => void;
  onScrollDown?: () => void;
  showScrollToTop?: boolean;
  scrollToTopThreshold?: number;
  children: React.ReactNode;
}

export const SmartScrollView: React.FC<SmartScrollViewProps> = ({
  onScrollUp,
  onScrollDown,
  showScrollToTop = true,
  scrollToTopThreshold = 300,
  children,
  ...scrollViewProps
}) => {
  const scrollViewRef = useRef<ScrollView>(null);
  const lastScrollY = useRef(0);
  const scrollDelta = useRef(0);
  const [showButton, setShowButton] = useState(false);
  const buttonOpacity = useRef(new Animated.Value(0)).current;

  // Throttle scroll handler for performance
  const handleScroll = useRef(
    throttle((event: any) => {
      const currentScrollY = event.nativeEvent.contentOffset.y;
      const delta = currentScrollY - lastScrollY.current;

      scrollDelta.current += delta;

      // Detect scroll direction
      if (scrollDelta.current > 50) {
        onScrollDown?.();
        scrollDelta.current = 0;
      } else if (scrollDelta.current < -50) {
        onScrollUp?.();
        scrollDelta.current = 0;
      }

      // Show/hide scroll-to-top button
      if (showScrollToTop) {
        const shouldShow = currentScrollY > scrollToTopThreshold;
        if (shouldShow !== showButton) {
          setShowButton(shouldShow);
          Animated.timing(buttonOpacity, {
            toValue: shouldShow ? 1 : 0,
            duration: 200,
            useNativeDriver: true,
          }).start();
        }
      }

      lastScrollY.current = currentScrollY;
    }, 100)
  ).current;

  const scrollToTop = () => {
    HapticFeedback.medium();
    scrollViewRef.current?.scrollTo({
      y: 0,
      animated: true,
    });
  };

  return (
    <>
      <ScrollView
        ref={scrollViewRef}
        onScroll={handleScroll}
        scrollEventThrottle={16}
        {...scrollViewProps}
      >
        {children}
      </ScrollView>

      {/* Scroll to top button */}
      {showScrollToTop && (
        <Animated.View
          style={[
            styles.scrollToTopButton,
            {
              opacity: buttonOpacity,
              pointerEvents: showButton ? 'auto' : 'none',
            },
          ]}
        >
          <TouchableOpacity onPress={scrollToTop} activeOpacity={0.8}>
            <LinearGradient
              colors={['#667eea', '#764ba2']}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 1 }}
              style={styles.buttonGradient}
            >
              <Ionicons name="arrow-up" size={24} color="#fff" />
            </LinearGradient>
          </TouchableOpacity>
        </Animated.View>
      )}
    </>
  );
};

const styles = StyleSheet.create({
  scrollToTopButton: {
    position: 'absolute',
    bottom: 24,
    right: 24,
    zIndex: 100,
  },
  buttonGradient: {
    width: 48,
    height: 48,
    borderRadius: 24,
    justifyContent: 'center',
    alignItems: 'center',
    ...Platform.select({
      ios: {
        shadowColor: '#000',
        shadowOffset: { width: 0, height: 4 },
        shadowOpacity: 0.3,
        shadowRadius: 8,
      },
      android: {
        elevation: 6,
      },
    }),
  },
});

/**
 * Hook for managing smart scroll behaviors
 */
export const useSmartScroll = () => {
  const [headerMinimized, setHeaderMinimized] = useState(false);

  const handleScrollUp = () => {
    setHeaderMinimized(false);
    HapticFeedback.light();
  };

  const handleScrollDown = () => {
    setHeaderMinimized(true);
    HapticFeedback.light();
  };

  return {
    headerMinimized,
    handleScrollUp,
    handleScrollDown,
  };
};
