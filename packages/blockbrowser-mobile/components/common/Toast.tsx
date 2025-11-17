/**
 * Toast - Premium toast notification system
 */

import React, { useRef, useEffect } from 'react';
import {
  View,
  Text,
  Animated,
  StyleSheet,
  useColorScheme,
  Platform,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { LinearGradient } from 'expo-linear-gradient';
import { Colors } from '@/constants/Colors';
import { HapticFeedback } from '@/lib/utils/haptics';

export type ToastType = 'success' | 'error' | 'warning' | 'info';

interface ToastProps {
  visible: boolean;
  message: string;
  type?: ToastType;
  duration?: number;
  onHide?: () => void;
  icon?: string;
}

export const Toast: React.FC<ToastProps> = ({
  visible,
  message,
  type = 'info',
  duration = 3000,
  onHide,
  icon,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;

  const translateY = useRef(new Animated.Value(-100)).current;
  const opacity = useRef(new Animated.Value(0)).current;

  useEffect(() => {
    if (visible) {
      // Show toast
      HapticFeedback.light();

      Animated.parallel([
        Animated.spring(translateY, {
          toValue: 0,
          tension: 300,
          friction: 30,
          useNativeDriver: true,
        }),
        Animated.timing(opacity, {
          toValue: 1,
          duration: 200,
          useNativeDriver: true,
        }),
      ]).start();

      // Auto-hide after duration
      const timer = setTimeout(() => {
        hideToast();
      }, duration);

      return () => clearTimeout(timer);
    }
  }, [visible]);

  const hideToast = () => {
    Animated.parallel([
      Animated.timing(translateY, {
        toValue: -100,
        duration: 200,
        useNativeDriver: true,
      }),
      Animated.timing(opacity, {
        toValue: 0,
        duration: 200,
        useNativeDriver: true,
      }),
    ]).start(() => {
      onHide?.();
    });
  };

  const getToastColors = () => {
    switch (type) {
      case 'success':
        return ['#10b981', '#059669'];
      case 'error':
        return ['#ef4444', '#dc2626'];
      case 'warning':
        return ['#f59e0b', '#d97706'];
      case 'info':
      default:
        return ['#667eea', '#764ba2'];
    }
  };

  const getToastIcon = () => {
    if (icon) return icon;

    switch (type) {
      case 'success':
        return 'checkmark-circle';
      case 'error':
        return 'close-circle';
      case 'warning':
        return 'warning';
      case 'info':
      default:
        return 'information-circle';
    }
  };

  if (!visible) return null;

  return (
    <Animated.View
      style={[
        styles.container,
        {
          transform: [{ translateY }],
          opacity,
        },
      ]}
    >
      <LinearGradient
        colors={getToastColors()}
        start={{ x: 0, y: 0 }}
        end={{ x: 1, y: 0 }}
        style={styles.toast}
      >
        <Ionicons name={getToastIcon() as any} size={24} color="#fff" />
        <Text style={styles.message} numberOfLines={2}>
          {message}
        </Text>
      </LinearGradient>
    </Animated.View>
  );
};

const styles = StyleSheet.create({
  container: {
    position: 'absolute',
    top: Platform.OS === 'ios' ? 60 : 20,
    left: 16,
    right: 16,
    zIndex: 9999,
  },
  toast: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 14,
    borderRadius: 12,
    gap: 12,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.2,
    shadowRadius: 8,
    elevation: 6,
  },
  message: {
    flex: 1,
    color: '#fff',
    fontSize: 15,
    fontWeight: '600',
  },
});

/**
 * Toast manager for showing toasts imperatively
 */
class ToastManager {
  private static instance: ToastManager;
  private listener?: (config: ToastConfig) => void;

  static getInstance(): ToastManager {
    if (!ToastManager.instance) {
      ToastManager.instance = new ToastManager();
    }
    return ToastManager.instance;
  }

  setListener(listener: (config: ToastConfig) => void) {
    this.listener = listener;
  }

  show(config: ToastConfig) {
    this.listener?.(config);
  }

  success(message: string, duration?: number) {
    this.show({ message, type: 'success', duration });
  }

  error(message: string, duration?: number) {
    this.show({ message, type: 'error', duration });
  }

  warning(message: string, duration?: number) {
    this.show({ message, type: 'warning', duration });
  }

  info(message: string, duration?: number) {
    this.show({ message, type: 'info', duration });
  }
}

interface ToastConfig {
  message: string;
  type?: ToastType;
  duration?: number;
  icon?: string;
}

export const toast = ToastManager.getInstance();

/**
 * ToastContainer - Add this to your app root
 */
export const ToastContainer: React.FC = () => {
  const [config, setConfig] = React.useState<ToastConfig | null>(null);
  const [visible, setVisible] = React.useState(false);

  useEffect(() => {
    toast.setListener((newConfig) => {
      setConfig(newConfig);
      setVisible(true);
    });
  }, []);

  return (
    <Toast
      visible={visible}
      message={config?.message || ''}
      type={config?.type}
      duration={config?.duration}
      icon={config?.icon}
      onHide={() => setVisible(false)}
    />
  );
};
