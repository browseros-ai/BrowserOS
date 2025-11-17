/**
 * Gesture Navigation System
 * Premium iOS-style navigation gestures
 */

import { useRef, useEffect } from 'react';
import {
  Animated,
  PanResponder,
  Dimensions,
  PanResponderGestureState,
} from 'react-native';
import { HapticFeedback } from './haptics';

const SCREEN_WIDTH = Dimensions.get('window').width;
const SWIPE_THRESHOLD = 50;
const EDGE_THRESHOLD = 20;

interface GestureNavigationProps {
  onSwipeBack?: () => void;
  onSwipeForward?: () => void;
  onPinchZoomOut?: () => void;
  enabled?: boolean;
}

/**
 * Hook for handling navigation gestures
 */
export const useGestureNavigation = ({
  onSwipeBack,
  onSwipeForward,
  onPinchZoomOut,
  enabled = true,
}: GestureNavigationProps) => {
  const translateX = useRef(new Animated.Value(0)).current;
  const lastTouchX = useRef(0);
  const isEdgeSwipe = useRef(false);

  const panResponder = useRef(
    PanResponder.create({
      onMoveShouldSetPanResponder: (evt, gestureState) => {
        if (!enabled) return false;

        const touchX = evt.nativeEvent.pageX;
        const isLeftEdge = touchX < EDGE_THRESHOLD;
        const isRightEdge = touchX > SCREEN_WIDTH - EDGE_THRESHOLD;

        // Only respond to edge swipes
        if (isLeftEdge || isRightEdge) {
          isEdgeSwipe.current = true;
          lastTouchX.current = touchX;
          return Math.abs(gestureState.dx) > 10;
        }

        return false;
      },

      onPanResponderGrant: () => {
        HapticFeedback.selection();
      },

      onPanResponderMove: (_, gestureState) => {
        if (!isEdgeSwipe.current) return;

        // Clamp the translation
        const newValue = Math.max(0, Math.min(SCREEN_WIDTH, gestureState.dx));
        translateX.setValue(newValue);

        // Haptic feedback at threshold
        if (Math.abs(gestureState.dx) > SWIPE_THRESHOLD) {
          HapticFeedback.light();
        }
      },

      onPanResponderRelease: (_, gestureState) => {
        if (!isEdgeSwipe.current) {
          isEdgeSwipe.current = false;
          return;
        }

        // Swipe from left edge = go back
        if (lastTouchX.current < EDGE_THRESHOLD && gestureState.dx > SWIPE_THRESHOLD) {
          HapticFeedback.medium();
          onSwipeBack?.();
        }
        // Swipe from right edge = go forward
        else if (
          lastTouchX.current > SCREEN_WIDTH - EDGE_THRESHOLD &&
          gestureState.dx < -SWIPE_THRESHOLD
        ) {
          HapticFeedback.medium();
          onSwipeForward?.();
        }

        // Reset
        Animated.spring(translateX, {
          toValue: 0,
          useNativeDriver: true,
          tension: 300,
          friction: 30,
        }).start();

        isEdgeSwipe.current = false;
      },

      onPanResponderTerminate: () => {
        // Reset on gesture cancel
        Animated.spring(translateX, {
          toValue: 0,
          useNativeDriver: true,
        }).start();
        isEdgeSwipe.current = false;
      },
    })
  ).current;

  return {
    panResponder,
    translateX,
  };
};

/**
 * Hook for pinch-to-zoom-out gesture (shows tab overview)
 */
export const usePinchGesture = (onPinchOut?: () => void, enabled = true) => {
  const scale = useRef(new Animated.Value(1)).current;
  const lastScale = useRef(1);

  const panResponder = useRef(
    PanResponder.create({
      onMoveShouldSetPanResponder: (_, gestureState) => {
        if (!enabled) return false;
        // Detect pinch (two fingers moving apart or together)
        return gestureState.numberActiveTouches === 2;
      },

      onPanResponderGrant: () => {
        HapticFeedback.selection();
      },

      onPanResponderMove: (evt, gestureState) => {
        if (gestureState.numberActiveTouches !== 2) return;

        // Calculate pinch scale
        const touches = evt.nativeEvent.touches;
        if (touches.length !== 2) return;

        const touch1 = touches[0];
        const touch2 = touches[1];

        const distance = Math.sqrt(
          Math.pow(touch2.pageX - touch1.pageX, 2) +
            Math.pow(touch2.pageY - touch1.pageY, 2)
        );

        // Simple scale detection (pinch out = zoom out)
        const newScale = distance / 200; // Normalize

        if (newScale < 0.7 && lastScale.current >= 0.7) {
          HapticFeedback.medium();
          onPinchOut?.();
        }

        lastScale.current = newScale;
        scale.setValue(newScale);
      },

      onPanResponderRelease: () => {
        // Reset
        Animated.spring(scale, {
          toValue: 1,
          useNativeDriver: true,
          tension: 300,
          friction: 30,
        }).start();

        lastScale.current = 1;
      },
    })
  ).current;

  return {
    panResponder,
    scale,
  };
};

/**
 * Hook for scroll-based gestures (minimizing address bar, etc.)
 */
export const useScrollGesture = (
  onScrollUp?: () => void,
  onScrollDown?: () => void
) => {
  const lastScrollY = useRef(0);
  const scrollDelta = useRef(0);

  const handleScroll = (event: any) => {
    const currentScrollY = event.nativeEvent.contentOffset.y;
    const delta = currentScrollY - lastScrollY.current;

    scrollDelta.current += delta;

    // Threshold for triggering callbacks
    if (scrollDelta.current > 50) {
      onScrollDown?.();
      scrollDelta.current = 0;
      HapticFeedback.light();
    } else if (scrollDelta.current < -50) {
      onScrollUp?.();
      scrollDelta.current = 0;
      HapticFeedback.light();
    }

    lastScrollY.current = currentScrollY;
  };

  return { handleScroll };
};

/**
 * Two-finger swipe for tab switching
 */
export const useTwoFingerSwipe = (
  onSwipeLeft?: () => void,
  onSwipeRight?: () => void,
  enabled = true
) => {
  const panResponder = useRef(
    PanResponder.create({
      onMoveShouldSetPanResponder: (_, gestureState) => {
        if (!enabled) return false;
        return (
          gestureState.numberActiveTouches === 2 &&
          Math.abs(gestureState.dx) > 20
        );
      },

      onPanResponderGrant: () => {
        HapticFeedback.selection();
      },

      onPanResponderRelease: (_, gestureState) => {
        if (gestureState.numberActiveTouches !== 2) return;

        if (gestureState.dx > SWIPE_THRESHOLD) {
          HapticFeedback.medium();
          onSwipeRight?.();
        } else if (gestureState.dx < -SWIPE_THRESHOLD) {
          HapticFeedback.medium();
          onSwipeLeft?.();
        }
      },
    })
  ).current;

  return { panResponder };
};
