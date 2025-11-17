/**
 * SmartAddressBar - Minimizes on scroll, expands on scroll up
 * Premium floating design
 */

import React, { useRef, useEffect } from 'react';
import {
  View,
  TextInput,
  TouchableOpacity,
  Animated,
  StyleSheet,
  useColorScheme,
  Platform,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { LinearGradient } from 'expo-linear-gradient';
import { Colors } from '@/constants/Colors';
import { HapticFeedback } from '@/lib/utils/haptics';

interface SmartAddressBarProps {
  url: string;
  onChangeUrl: (url: string) => void;
  onSubmit: () => void;
  onRefresh?: () => void;
  isLoading?: boolean;
  canGoBack?: boolean;
  canGoForward?: boolean;
  onGoBack?: () => void;
  onGoForward?: () => void;
  isMinimized?: boolean; // Controlled by scroll
}

export const SmartAddressBar: React.FC<SmartAddressBarProps> = ({
  url,
  onChangeUrl,
  onSubmit,
  onRefresh,
  isLoading = false,
  canGoBack = false,
  canGoForward = false,
  onGoBack,
  onGoForward,
  isMinimized = false,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;

  const heightAnim = useRef(new Animated.Value(1)).current;
  const opacityAnim = useRef(new Animated.Value(1)).current;

  useEffect(() => {
    Animated.parallel([
      Animated.spring(heightAnim, {
        toValue: isMinimized ? 0.6 : 1,
        useNativeDriver: false,
        tension: 300,
        friction: 30,
      }),
      Animated.timing(opacityAnim, {
        toValue: isMinimized ? 0.7 : 1,
        duration: 200,
        useNativeDriver: true,
      }),
    ]).start();
  }, [isMinimized]);

  const animatedHeight = heightAnim.interpolate({
    inputRange: [0.6, 1],
    outputRange: [48, 80],
  });

  const handleGoBack = () => {
    if (!canGoBack) return;
    HapticFeedback.light();
    onGoBack?.();
  };

  const handleGoForward = () => {
    if (!canGoForward) return;
    HapticFeedback.light();
    onGoForward?.();
  };

  const handleRefresh = () => {
    HapticFeedback.medium();
    onRefresh?.();
  };

  return (
    <Animated.View
      style={[
        styles.container,
        {
          height: animatedHeight,
          opacity: opacityAnim,
        },
      ]}
    >
      <LinearGradient
        colors={['#667eea', '#764ba2']}
        start={{ x: 0, y: 0 }}
        end={{ x: 1, y: 0 }}
        style={styles.gradient}
      >
        <View style={styles.content}>
          {/* Navigation buttons */}
          <View style={styles.navButtons}>
            <TouchableOpacity
              onPress={handleGoBack}
              disabled={!canGoBack}
              style={styles.navButton}
            >
              <Ionicons
                name="chevron-back"
                size={24}
                color={canGoBack ? '#fff' : 'rgba(255,255,255,0.3)'}
              />
            </TouchableOpacity>

            <TouchableOpacity
              onPress={handleGoForward}
              disabled={!canGoForward}
              style={styles.navButton}
            >
              <Ionicons
                name="chevron-forward"
                size={24}
                color={canGoForward ? '#fff' : 'rgba(255,255,255,0.3)'}
              />
            </TouchableOpacity>
          </View>

          {/* URL Input */}
          <View style={[styles.urlContainer, { backgroundColor: 'rgba(255,255,255,0.2)' }]}>
            <Ionicons
              name={isLoading ? 'hourglass-outline' : 'globe-outline'}
              size={18}
              color="#fff"
              style={styles.urlIcon}
            />
            <TextInput
              value={url}
              onChangeText={onChangeUrl}
              onSubmitEditing={onSubmit}
              placeholder="Search or enter URL..."
              placeholderTextColor="rgba(255,255,255,0.6)"
              style={styles.urlInput}
              autoCapitalize="none"
              autoCorrect={false}
              keyboardType="url"
              returnKeyType="go"
              selectTextOnFocus
            />
          </View>

          {/* Refresh button */}
          <TouchableOpacity onPress={handleRefresh} style={styles.refreshButton}>
            <Ionicons
              name={isLoading ? 'close' : 'refresh'}
              size={24}
              color="#fff"
            />
          </TouchableOpacity>
        </View>
      </LinearGradient>

      {/* Shadow */}
      <View style={[styles.shadow, { backgroundColor: colors.background }]} />
    </Animated.View>
  );
};

const styles = StyleSheet.create({
  container: {
    width: '100%',
    overflow: 'hidden',
    ...Platform.select({
      ios: {
        shadowColor: '#000',
        shadowOffset: { width: 0, height: 2 },
        shadowOpacity: 0.1,
        shadowRadius: 8,
      },
      android: {
        elevation: 4,
      },
    }),
  },
  gradient: {
    flex: 1,
    paddingTop: Platform.OS === 'ios' ? 44 : 0, // Status bar padding
  },
  content: {
    flex: 1,
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 12,
    gap: 8,
  },
  navButtons: {
    flexDirection: 'row',
    gap: 4,
  },
  navButton: {
    width: 36,
    height: 36,
    justifyContent: 'center',
    alignItems: 'center',
    borderRadius: 18,
  },
  urlContainer: {
    flex: 1,
    flexDirection: 'row',
    alignItems: 'center',
    height: 40,
    borderRadius: 20,
    paddingHorizontal: 12,
    gap: 8,
  },
  urlIcon: {
    marginRight: 4,
  },
  urlInput: {
    flex: 1,
    color: '#fff',
    fontSize: 15,
    fontWeight: '500',
  },
  refreshButton: {
    width: 40,
    height: 40,
    justifyContent: 'center',
    alignItems: 'center',
    borderRadius: 20,
  },
  shadow: {
    position: 'absolute',
    bottom: -8,
    left: 0,
    right: 0,
    height: 8,
    opacity: 0.05,
  },
});
