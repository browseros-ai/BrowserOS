import React from 'react';
import { View, Text, TouchableOpacity, StyleSheet, useColorScheme } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { Colors } from '@/constants/Colors';

interface ErrorPageProps {
  error: string;
  url: string;
  onRetry: () => void;
  onGoHome: () => void;
}

export const ErrorPage: React.FC<ErrorPageProps> = ({
  error,
  url,
  onRetry,
  onGoHome,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;

  return (
    <View style={[styles.container, { backgroundColor: colors.background }]}>
      <View style={styles.content}>
        {/* Error Icon */}
        <View
          style={[
            styles.iconContainer,
            { backgroundColor: colors.error + '20' },
          ]}
        >
          <Ionicons name="alert-circle" size={64} color={colors.error} />
        </View>

        {/* Error Title */}
        <Text style={[styles.title, { color: colors.text }]}>
          Page Failed to Load
        </Text>

        {/* Error Message */}
        <Text style={[styles.message, { color: colors.textSecondary }]}>
          {error || 'An unknown error occurred while loading this page.'}
        </Text>

        {/* URL */}
        <View
          style={[
            styles.urlContainer,
            {
              backgroundColor: colors.surface,
              borderColor: colors.border,
            },
          ]}
        >
          <Ionicons name="link" size={16} color={colors.textSecondary} />
          <Text
            style={[styles.url, { color: colors.textSecondary }]}
            numberOfLines={2}
          >
            {url}
          </Text>
        </View>

        {/* Action Buttons */}
        <View style={styles.actions}>
          <TouchableOpacity
            style={[
              styles.button,
              styles.primaryButton,
              { backgroundColor: colors.primary },
            ]}
            onPress={onRetry}
          >
            <Ionicons name="reload" size={20} color={colors.background} />
            <Text style={[styles.buttonText, { color: colors.background }]}>
              Try Again
            </Text>
          </TouchableOpacity>

          <TouchableOpacity
            style={[
              styles.button,
              styles.secondaryButton,
              {
                backgroundColor: colors.surface,
                borderColor: colors.border,
              },
            ]}
            onPress={onGoHome}
          >
            <Ionicons name="home" size={20} color={colors.primary} />
            <Text style={[styles.buttonText, { color: colors.primary }]}>
              Go Home
            </Text>
          </TouchableOpacity>
        </View>

        {/* Help Text */}
        <View style={styles.helpContainer}>
          <Text style={[styles.helpTitle, { color: colors.text }]}>
            Common causes:
          </Text>
          <Text style={[styles.helpText, { color: colors.textSecondary }]}>
            • No internet connection
          </Text>
          <Text style={[styles.helpText, { color: colors.textSecondary }]}>
            • The website is down or moved
          </Text>
          <Text style={[styles.helpText, { color: colors.textSecondary }]}>
            • Invalid or mistyped URL
          </Text>
          <Text style={[styles.helpText, { color: colors.textSecondary }]}>
            • Network or firewall restrictions
          </Text>
        </View>
      </View>
    </View>
  );
};

const styles = StyleSheet.create({
  container: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
    padding: 24,
  },
  content: {
    width: '100%',
    maxWidth: 400,
    alignItems: 'center',
  },
  iconContainer: {
    width: 120,
    height: 120,
    borderRadius: 60,
    alignItems: 'center',
    justifyContent: 'center',
    marginBottom: 24,
  },
  title: {
    fontSize: 24,
    fontWeight: '700',
    textAlign: 'center',
    marginBottom: 12,
  },
  message: {
    fontSize: 16,
    textAlign: 'center',
    lineHeight: 24,
    marginBottom: 24,
  },
  urlContainer: {
    flexDirection: 'row',
    alignItems: 'center',
    padding: 12,
    borderRadius: 8,
    borderWidth: 1,
    gap: 8,
    width: '100%',
    marginBottom: 32,
  },
  url: {
    flex: 1,
    fontSize: 13,
    fontFamily: 'monospace',
  },
  actions: {
    flexDirection: 'row',
    gap: 12,
    width: '100%',
    marginBottom: 32,
  },
  button: {
    flex: 1,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    paddingVertical: 14,
    borderRadius: 12,
    gap: 8,
  },
  primaryButton: {
    // backgroundColor set dynamically
  },
  secondaryButton: {
    borderWidth: 2,
  },
  buttonText: {
    fontSize: 16,
    fontWeight: '600',
  },
  helpContainer: {
    width: '100%',
    alignItems: 'flex-start',
  },
  helpTitle: {
    fontSize: 14,
    fontWeight: '600',
    marginBottom: 8,
  },
  helpText: {
    fontSize: 14,
    lineHeight: 20,
    marginBottom: 4,
  },
});
