import React from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  StyleSheet,
  useColorScheme,
  Platform,
  Animated,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { Colors } from '@/constants/Colors';

interface QuickAction {
  icon: string;
  label: string;
  color: string;
  onPress: () => void;
}

interface QuickActionsBarProps {
  url: string;
  onAskAI: () => void;
  onSummarize: () => void;
  onTranslate: () => void;
  onReaderMode: () => void;
  onShare: () => void;
}

export const QuickActionsBar: React.FC<QuickActionsBarProps> = ({
  url,
  onAskAI,
  onSummarize,
  onTranslate,
  onReaderMode,
  onShare,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;

  const actions: QuickAction[] = [
    {
      icon: 'sparkles',
      label: 'Ask AI',
      color: '#667eea',
      onPress: onAskAI,
    },
    {
      icon: 'document-text',
      label: 'TL;DR',
      color: '#f59e0b',
      onPress: onSummarize,
    },
    {
      icon: 'language',
      label: 'Translate',
      color: '#10b981',
      onPress: onTranslate,
    },
    {
      icon: 'book',
      label: 'Reader',
      color: '#3b82f6',
      onPress: onReaderMode,
    },
    {
      icon: 'share-outline',
      label: 'Share',
      color: '#8b5cf6',
      onPress: onShare,
    },
  ];

  return (
    <View
      style={[
        styles.container,
        {
          backgroundColor: colors.surface,
          borderTopColor: colors.border,
        },
      ]}
    >
      {actions.map((action, index) => (
        <TouchableOpacity
          key={index}
          style={styles.action}
          onPress={action.onPress}
          activeOpacity={0.7}
        >
          <View
            style={[
              styles.iconContainer,
              { backgroundColor: action.color + '20' },
            ]}
          >
            <Ionicons
              name={action.icon as any}
              size={20}
              color={action.color}
            />
          </View>
          <Text style={[styles.label, { color: colors.textSecondary }]}>
            {action.label}
          </Text>
        </TouchableOpacity>
      ))}
    </View>
  );
};

const styles = StyleSheet.create({
  container: {
    flexDirection: 'row',
    paddingVertical: 12,
    paddingHorizontal: 8,
    borderTopWidth: 1,
    justifyContent: 'space-around',
    shadowColor: '#000',
    shadowOffset: { width: 0, height: -2 },
    shadowOpacity: 0.1,
    shadowRadius: 4,
    elevation: 8,
  },
  action: {
    alignItems: 'center',
    gap: 6,
    flex: 1,
  },
  iconContainer: {
    width: 44,
    height: 44,
    borderRadius: 22,
    alignItems: 'center',
    justifyContent: 'center',
  },
  label: {
    fontSize: 11,
    fontWeight: '600',
  },
});
