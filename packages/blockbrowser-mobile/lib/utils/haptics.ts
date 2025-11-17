/**
 * Haptic Feedback Utilities
 * Premium tactile feedback for all interactions
 */

import * as Haptics from 'expo-haptics';
import { Platform } from 'react-native';

export class HapticFeedback {
  /**
   * Light tap - For button presses, selections
   */
  static light() {
    if (Platform.OS === 'ios') {
      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    }
  }

  /**
   * Medium tap - For switches, toggles
   */
  static medium() {
    if (Platform.OS === 'ios') {
      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    }
  }

  /**
   * Heavy tap - For important actions, confirmations
   */
  static heavy() {
    if (Platform.OS === 'ios') {
      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Heavy);
    }
  }

  /**
   * Success - For successful operations
   */
  static success() {
    if (Platform.OS === 'ios') {
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
    }
  }

  /**
   * Warning - For warnings
   */
  static warning() {
    if (Platform.OS === 'ios') {
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Warning);
    }
  }

  /**
   * Error - For errors
   */
  static error() {
    if (Platform.OS === 'ios') {
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Error);
    }
  }

  /**
   * Selection - For scrolling through values
   */
  static selection() {
    if (Platform.OS === 'ios') {
      Haptics.selectionAsync();
    }
  }

  /**
   * Custom pattern - For special interactions
   */
  static custom(pattern: 'tick' | 'snap' | 'bounce') {
    if (Platform.OS === 'ios') {
      switch (pattern) {
        case 'tick':
          Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          break;
        case 'snap':
          Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Heavy);
          setTimeout(() => {
            Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          }, 50);
          break;
        case 'bounce':
          Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
          setTimeout(() => {
            Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          }, 100);
          break;
      }
    }
  }
}
