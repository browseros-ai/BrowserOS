import React, { useState, useEffect } from 'react';
import {
  View,
  TextInput,
  TouchableOpacity,
  StyleSheet,
  Platform,
  useColorScheme,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { LiquidGlassView, isLiquidGlassSupported } from '@callstack/liquid-glass';
import { URLValidator } from '@/lib/utils/url-validator';
import { Colors } from '@/constants/Colors';

interface AddressBarProps {
  url: string;
  isLoading: boolean;
  isSecure: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
  onNavigate: (url: string) => void;
  onGoBack: () => void;
  onGoForward: () => void;
  onReload: () => void;
  onShare: () => void;
}

export const AddressBar: React.FC<AddressBarProps> = ({
  url,
  isLoading,
  isSecure,
  canGoBack,
  canGoForward,
  onNavigate,
  onGoBack,
  onGoForward,
  onReload,
  onShare,
}) => {
  const colorScheme = useColorScheme();
  const [inputValue, setInputValue] = useState(url);
  const [isFocused, setIsFocused] = useState(false);
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;

  // Check if we can use liquid glass (iOS only)
  const useLiquidGlass = Platform.OS === 'ios' && isLiquidGlassSupported();

  useEffect(() => {
    if (!isFocused) {
      setInputValue(url);
    }
  }, [url, isFocused]);

  const handleSubmit = () => {
    const normalizedUrl = URLValidator.normalizeInput(inputValue);
    onNavigate(normalizedUrl);
    setIsFocused(false);
  };

  const displayUrl = isFocused
    ? inputValue
    : URLValidator.shortenURL(url, 40);

  const AddressBarContent = () => (
    <View style={styles.container}>
      {/* Back Button */}
      <TouchableOpacity
        style={[styles.navButton, !canGoBack && styles.navButtonDisabled]}
        onPress={onGoBack}
        disabled={!canGoBack}
        activeOpacity={0.7}
      >
        <Ionicons
          name="chevron-back"
          size={24}
          color={canGoBack ? colors.primary : colors.textSecondary}
        />
      </TouchableOpacity>

      {/* Forward Button */}
      <TouchableOpacity
        style={[styles.navButton, !canGoForward && styles.navButtonDisabled]}
        onPress={onGoForward}
        disabled={!canGoForward}
        activeOpacity={0.7}
      >
        <Ionicons
          name="chevron-forward"
          size={24}
          color={canGoForward ? colors.primary : colors.textSecondary}
        />
      </TouchableOpacity>

      {/* URL Input */}
      <View
        style={[
          styles.urlContainer,
          {
            backgroundColor: colorScheme === 'dark' ? 'rgba(255,255,255,0.1)' : 'rgba(0,0,0,0.05)',
          },
          isFocused && styles.urlContainerFocused,
        ]}
      >
        {/* Lock Icon */}
        <Ionicons
          name={isSecure ? 'lock-closed' : 'lock-open'}
          size={16}
          color={isSecure ? colors.success : colors.warning}
          style={styles.lockIcon}
        />

        {/* URL Input Field */}
        <TextInput
          style={[styles.urlInput, { color: colors.text }]}
          value={isFocused ? inputValue : displayUrl}
          onChangeText={setInputValue}
          onFocus={() => setIsFocused(true)}
          onBlur={() => setIsFocused(false)}
          onSubmitEditing={handleSubmit}
          placeholder="Search or enter URL"
          placeholderTextColor={colors.textSecondary}
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType="url"
          returnKeyType="go"
          selectTextOnFocus={true}
        />

        {/* Loading/Clear Icon */}
        {isFocused && inputValue ? (
          <TouchableOpacity
            onPress={() => setInputValue('')}
            hitSlop={{ top: 10, bottom: 10, left: 10, right: 10 }}
          >
            <Ionicons name="close-circle" size={20} color={colors.textSecondary} />
          </TouchableOpacity>
        ) : null}
      </View>

      {/* Reload/Stop Button */}
      <TouchableOpacity style={styles.actionButton} onPress={onReload} activeOpacity={0.7}>
        <Ionicons
          name={isLoading ? 'close' : 'reload'}
          size={24}
          color={colors.primary}
        />
      </TouchableOpacity>

      {/* Share Button */}
      <TouchableOpacity style={styles.actionButton} onPress={onShare} activeOpacity={0.7}>
        <Ionicons name="share-outline" size={24} color={colors.primary} />
      </TouchableOpacity>
    </View>
  );

  if (useLiquidGlass) {
    return (
      <LiquidGlassView
        style={styles.glassContainer}
        effect="regular"
        tintColor={colorScheme === 'dark' ? '#1a1a1b' : '#ffffff'}
        colorScheme={colorScheme || 'light'}
      >
        <AddressBarContent />
      </LiquidGlassView>
    );
  }

  // Fallback for Android and older iOS versions
  return (
    <View
      style={[
        styles.glassContainer,
        styles.fallbackGlass,
        { backgroundColor: colors.surface },
      ]}
    >
      <AddressBarContent />
    </View>
  );
};

const styles = StyleSheet.create({
  glassContainer: {
    paddingHorizontal: 12,
    paddingVertical: 8,
    paddingTop: Platform.OS === 'ios' ? 50 : 8, // Account for notch
  },
  fallbackGlass: {
    borderBottomWidth: 1,
    borderBottomColor: 'rgba(0, 0, 0, 0.1)',
  },
  container: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
  },
  navButton: {
    padding: 8,
    borderRadius: 8,
  },
  navButtonDisabled: {
    opacity: 0.3,
  },
  urlContainer: {
    flex: 1,
    flexDirection: 'row',
    alignItems: 'center',
    backgroundColor: 'rgba(0, 0, 0, 0.05)',
    borderRadius: 12,
    paddingHorizontal: 12,
    paddingVertical: 10,
    gap: 8,
    borderWidth: 1,
    borderColor: 'transparent',
  },
  urlContainerFocused: {
    borderColor: '#667eea',
  },
  lockIcon: {
    marginRight: 4,
  },
  urlInput: {
    flex: 1,
    fontSize: 15,
    fontFamily: Platform.OS === 'ios' ? 'System' : 'Roboto',
  },
  actionButton: {
    padding: 8,
    borderRadius: 8,
  },
});
