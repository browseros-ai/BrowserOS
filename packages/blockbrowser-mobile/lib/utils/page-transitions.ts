/**
 * Page Transition System
 * Smooth animations for screen transitions
 */

import { Animated, Easing } from 'react-native';

export type TransitionType = 'slide' | 'fade' | 'scale' | 'slideUp' | 'slideDown';

export interface TransitionConfig {
  duration?: number;
  easing?: any;
  useNativeDriver?: boolean;
}

const DEFAULT_CONFIG: TransitionConfig = {
  duration: 300,
  easing: Easing.bezier(0.25, 0.1, 0.25, 1), // Material Design easing
  useNativeDriver: true,
};

/**
 * Create slide transition (left to right)
 */
export const createSlideTransition = (
  animatedValue: Animated.Value,
  config: TransitionConfig = {}
) => {
  const finalConfig = { ...DEFAULT_CONFIG, ...config };

  return {
    in: () => {
      animatedValue.setValue(100);
      return Animated.timing(animatedValue, {
        toValue: 0,
        duration: finalConfig.duration,
        easing: finalConfig.easing,
        useNativeDriver: finalConfig.useNativeDriver!,
      });
    },
    out: () => {
      return Animated.timing(animatedValue, {
        toValue: -100,
        duration: finalConfig.duration,
        easing: finalConfig.easing,
        useNativeDriver: finalConfig.useNativeDriver!,
      });
    },
  };
};

/**
 * Create fade transition
 */
export const createFadeTransition = (
  animatedValue: Animated.Value,
  config: TransitionConfig = {}
) => {
  const finalConfig = { ...DEFAULT_CONFIG, ...config };

  return {
    in: () => {
      animatedValue.setValue(0);
      return Animated.timing(animatedValue, {
        toValue: 1,
        duration: finalConfig.duration,
        easing: finalConfig.easing,
        useNativeDriver: finalConfig.useNativeDriver!,
      });
    },
    out: () => {
      return Animated.timing(animatedValue, {
        toValue: 0,
        duration: finalConfig.duration,
        easing: finalConfig.easing,
        useNativeDriver: finalConfig.useNativeDriver!,
      });
    },
  };
};

/**
 * Create scale transition (zoom in/out)
 */
export const createScaleTransition = (
  animatedValue: Animated.Value,
  config: TransitionConfig = {}
) => {
  const finalConfig = { ...DEFAULT_CONFIG, ...config };

  return {
    in: () => {
      animatedValue.setValue(0.8);
      return Animated.spring(animatedValue, {
        toValue: 1,
        tension: 300,
        friction: 30,
        useNativeDriver: finalConfig.useNativeDriver!,
      });
    },
    out: () => {
      return Animated.spring(animatedValue, {
        toValue: 0.8,
        tension: 300,
        friction: 30,
        useNativeDriver: finalConfig.useNativeDriver!,
      });
    },
  };
};

/**
 * Create slide up transition (bottom to top)
 */
export const createSlideUpTransition = (
  animatedValue: Animated.Value,
  config: TransitionConfig = {}
) => {
  const finalConfig = { ...DEFAULT_CONFIG, ...config };

  return {
    in: () => {
      animatedValue.setValue(100);
      return Animated.spring(animatedValue, {
        toValue: 0,
        tension: 300,
        friction: 30,
        useNativeDriver: finalConfig.useNativeDriver!,
      });
    },
    out: () => {
      return Animated.timing(animatedValue, {
        toValue: 100,
        duration: finalConfig.duration,
        easing: finalConfig.easing,
        useNativeDriver: finalConfig.useNativeDriver!,
      });
    },
  };
};

/**
 * Combined transition (fade + scale)
 */
export const createCombinedTransition = (
  opacity: Animated.Value,
  scale: Animated.Value,
  config: TransitionConfig = {}
) => {
  const finalConfig = { ...DEFAULT_CONFIG, ...config };

  return {
    in: () => {
      opacity.setValue(0);
      scale.setValue(0.9);

      return Animated.parallel([
        Animated.timing(opacity, {
          toValue: 1,
          duration: finalConfig.duration,
          easing: finalConfig.easing,
          useNativeDriver: finalConfig.useNativeDriver!,
        }),
        Animated.spring(scale, {
          toValue: 1,
          tension: 300,
          friction: 30,
          useNativeDriver: finalConfig.useNativeDriver!,
        }),
      ]);
    },
    out: () => {
      return Animated.parallel([
        Animated.timing(opacity, {
          toValue: 0,
          duration: finalConfig.duration! * 0.7, // Faster out
          easing: finalConfig.easing,
          useNativeDriver: finalConfig.useNativeDriver!,
        }),
        Animated.timing(scale, {
          toValue: 0.9,
          duration: finalConfig.duration! * 0.7,
          easing: finalConfig.easing,
          useNativeDriver: finalConfig.useNativeDriver!,
        }),
      ]);
    },
  };
};

/**
 * Card-style transition (for tab switching)
 */
export const createCardTransition = (
  translateY: Animated.Value,
  scale: Animated.Value,
  opacity: Animated.Value,
  config: TransitionConfig = {}
) => {
  const finalConfig = { ...DEFAULT_CONFIG, ...config };

  return {
    in: () => {
      translateY.setValue(50);
      scale.setValue(0.95);
      opacity.setValue(0);

      return Animated.parallel([
        Animated.spring(translateY, {
          toValue: 0,
          tension: 300,
          friction: 30,
          useNativeDriver: finalConfig.useNativeDriver!,
        }),
        Animated.spring(scale, {
          toValue: 1,
          tension: 300,
          friction: 30,
          useNativeDriver: finalConfig.useNativeDriver!,
        }),
        Animated.timing(opacity, {
          toValue: 1,
          duration: finalConfig.duration,
          easing: finalConfig.easing,
          useNativeDriver: finalConfig.useNativeDriver!,
        }),
      ]);
    },
    out: () => {
      return Animated.parallel([
        Animated.timing(translateY, {
          toValue: -50,
          duration: finalConfig.duration,
          easing: finalConfig.easing,
          useNativeDriver: finalConfig.useNativeDriver!,
        }),
        Animated.timing(scale, {
          toValue: 0.95,
          duration: finalConfig.duration,
          easing: finalConfig.easing,
          useNativeDriver: finalConfig.useNativeDriver!,
        }),
        Animated.timing(opacity, {
          toValue: 0,
          duration: finalConfig.duration,
          easing: finalConfig.easing,
          useNativeDriver: finalConfig.useNativeDriver!,
        }),
      ]);
    },
  };
};

/**
 * Spring-based bounce animation
 */
export const createBounceAnimation = (
  animatedValue: Animated.Value,
  toValue: number = 1
) => {
  return Animated.spring(animatedValue, {
    toValue,
    tension: 400,
    friction: 8,
    useNativeDriver: true,
  });
};

/**
 * Sequence multiple transitions
 */
export const sequenceTransitions = (...animations: Animated.CompositeAnimation[]) => {
  return Animated.sequence(animations);
};

/**
 * Stagger transitions (one after another with delay)
 */
export const staggerTransitions = (
  animations: Animated.CompositeAnimation[],
  delayMs: number = 50
) => {
  return Animated.stagger(delayMs, animations);
};
