import React, { useState, useEffect } from 'react';
import {
  View,
  Text,
  Modal,
  ScrollView,
  TouchableOpacity,
  StyleSheet,
  useColorScheme,
  ActivityIndicator,
  Platform,
} from 'react-native';
import { LinearGradient } from 'expo-linear-gradient';
import { Ionicons } from '@expo/vector-icons';
import { LiquidGlassView } from '@callstack/liquid-glass';
import { isLiquidGlassSupported } from '@/lib/utils/liquid-glass-helper';
import { Colors } from '@/constants/Colors';
import { AIProviderFactory } from '@/lib/ai/providers/factory';
import { SecureStorage } from '@/lib/storage/secure-store';
import { useSettingsStore } from '@/store/settings-store';

interface PageSummaryModalProps {
  visible: boolean;
  pageUrl: string;
  pageTitle: string;
  pageContent: string;
  onClose: () => void;
}

export const PageSummaryModal: React.FC<PageSummaryModalProps> = ({
  visible,
  pageUrl,
  pageTitle,
  pageContent,
  onClose,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;
  const useLiquidGlass = Platform.OS === 'ios' && isLiquidGlassSupported();

  const [summary, setSummary] = useState('');
  const [keyPoints, setKeyPoints] = useState<string[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [readingTime, setReadingTime] = useState('');

  const { aiProviders } = useSettingsStore();

  useEffect(() => {
    if (visible && pageContent) {
      generateSummary();
      calculateReadingTime();
    }
  }, [visible, pageContent]);

  const calculateReadingTime = () => {
    const words = pageContent.split(/\s+/).length;
    const minutes = Math.ceil(words / 200); // Average reading speed
    setReadingTime(`${minutes} min read`);
  };

  const generateSummary = async () => {
    setIsLoading(true);
    try {
      // Get first enabled AI provider
      const enabledProvider = aiProviders.find((p) => p.enabled);
      if (!enabledProvider) {
        setSummary('Please configure an AI provider in Settings to use this feature.');
        setIsLoading(false);
        return;
      }

      const provider = AIProviderFactory.getProvider(enabledProvider.type as any);
      const apiKey = await SecureStorage.getApiKey(enabledProvider.id);

      if (!apiKey) {
        setSummary('API key not found. Please configure it in Settings.');
        setIsLoading(false);
        return;
      }

      const prompt = `Summarize this webpage in 2-3 sentences, then provide 3-5 key points as bullet points.

Title: ${pageTitle}
URL: ${pageUrl}
Content: ${pageContent.slice(0, 4000)}

Format:
Summary: [2-3 sentences]

Key Points:
- Point 1
- Point 2
- Point 3`;

      const response = await provider.chat(
        [{ role: 'user', content: prompt }],
        enabledProvider.models[0] || 'gpt-4',
        apiKey
      );

      // Parse response
      const parts = response.split('Key Points:');
      const summaryText = parts[0].replace('Summary:', '').trim();
      const pointsText = parts[1] || '';
      const points = pointsText
        .split('\n')
        .filter((line) => line.trim().startsWith('-'))
        .map((line) => line.replace(/^-\s*/, '').trim());

      setSummary(summaryText);
      setKeyPoints(points);
    } catch (error) {
      console.error('Error generating summary:', error);
      setSummary('Failed to generate summary. Please try again.');
    } finally {
      setIsLoading(false);
    }
  };

  const ModalContent = () => (
    <View
      style={[
        styles.modalContent,
        { backgroundColor: colors.background },
      ]}
    >
      {/* Header */}
      <LinearGradient
        colors={['#667eea', '#764ba2']}
        start={{ x: 0, y: 0 }}
        end={{ x: 1, y: 1 }}
        style={styles.header}
      >
        <View style={styles.headerContent}>
          <View style={styles.headerLeft}>
            <View style={styles.iconContainer}>
              <Ionicons name="document-text" size={24} color="#fff" />
            </View>
            <View style={styles.headerText}>
              <Text style={styles.headerTitle}>Page Summary</Text>
              <Text style={styles.headerSubtitle}>
                {readingTime} • AI-generated
              </Text>
            </View>
          </View>
          <TouchableOpacity onPress={onClose} style={styles.closeButton}>
            <Ionicons name="close" size={28} color="#fff" />
          </TouchableOpacity>
        </View>
      </LinearGradient>

      {/* Content */}
      <ScrollView
        style={styles.scrollView}
        contentContainerStyle={styles.scrollContent}
        showsVerticalScrollIndicator={false}
      >
        {/* Page Info */}
        <View style={styles.pageInfo}>
          <Text
            style={[styles.pageTitle, { color: colors.text }]}
            numberOfLines={2}
          >
            {pageTitle}
          </Text>
          <Text
            style={[styles.pageUrl, { color: colors.textSecondary }]}
            numberOfLines={1}
          >
            {pageUrl}
          </Text>
        </View>

        {/* Loading */}
        {isLoading && (
          <View style={styles.loadingContainer}>
            <ActivityIndicator size="large" color={colors.primary} />
            <Text style={[styles.loadingText, { color: colors.textSecondary }]}>
              Analyzing page with AI...
            </Text>
          </View>
        )}

        {/* Summary */}
        {!isLoading && summary && (
          <>
            <View style={styles.section}>
              <View style={styles.sectionHeader}>
                <Ionicons name="newspaper" size={20} color={colors.primary} />
                <Text style={[styles.sectionTitle, { color: colors.text }]}>
                  Summary
                </Text>
              </View>
              <Text style={[styles.summaryText, { color: colors.text }]}>
                {summary}
              </Text>
            </View>

            {/* Key Points */}
            {keyPoints.length > 0 && (
              <View style={styles.section}>
                <View style={styles.sectionHeader}>
                  <Ionicons name="list" size={20} color={colors.primary} />
                  <Text style={[styles.sectionTitle, { color: colors.text }]}>
                    Key Points
                  </Text>
                </View>
                {keyPoints.map((point, index) => (
                  <View
                    key={index}
                    style={[
                      styles.keyPoint,
                      {
                        backgroundColor: colors.surface,
                        borderColor: colors.border,
                      },
                    ]}
                  >
                    <View
                      style={[
                        styles.bulletPoint,
                        { backgroundColor: colors.primary },
                      ]}
                    >
                      <Text style={styles.bulletText}>{index + 1}</Text>
                    </View>
                    <Text style={[styles.pointText, { color: colors.text }]}>
                      {point}
                    </Text>
                  </View>
                ))}
              </View>
            )}

            {/* Actions */}
            <View style={styles.actions}>
              <TouchableOpacity
                style={[
                  styles.actionButton,
                  {
                    backgroundColor: colors.surface,
                    borderColor: colors.border,
                  },
                ]}
                onPress={onClose}
              >
                <Ionicons name="checkmark" size={20} color={colors.primary} />
                <Text style={[styles.actionText, { color: colors.text }]}>
                  Got it
                </Text>
              </TouchableOpacity>
            </View>
          </>
        )}
      </ScrollView>
    </View>
  );

  return (
    <Modal
      visible={visible}
      animationType="slide"
      presentationStyle="pageSheet"
      onRequestClose={onClose}
    >
      {useLiquidGlass ? (
        <LiquidGlassView
          style={styles.modalGlass}
          effect="regular"
          tintColor={colorScheme === 'dark' ? '#000000' : '#f2f2f7'}
          colorScheme={colorScheme || 'light'}
        >
          <ModalContent />
        </LiquidGlassView>
      ) : (
        <ModalContent />
      )}
    </Modal>
  );
};

const styles = StyleSheet.create({
  modalGlass: {
    flex: 1,
  },
  modalContent: {
    flex: 1,
  },
  header: {
    paddingTop: Platform.OS === 'ios' ? 60 : 24,
    paddingBottom: 20,
    paddingHorizontal: 20,
    borderBottomLeftRadius: 24,
    borderBottomRightRadius: 24,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.3,
    shadowRadius: 8,
    elevation: 8,
  },
  headerContent: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
  },
  headerLeft: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 12,
    flex: 1,
  },
  iconContainer: {
    width: 48,
    height: 48,
    borderRadius: 12,
    backgroundColor: 'rgba(255, 255, 255, 0.2)',
    alignItems: 'center',
    justifyContent: 'center',
    borderWidth: 2,
    borderColor: 'rgba(255, 255, 255, 0.3)',
  },
  headerText: {
    flex: 1,
  },
  headerTitle: {
    fontSize: 20,
    fontWeight: '800',
    color: '#fff',
    letterSpacing: 0.5,
  },
  headerSubtitle: {
    fontSize: 13,
    color: 'rgba(255, 255, 255, 0.9)',
    marginTop: 2,
  },
  closeButton: {
    padding: 4,
  },
  scrollView: {
    flex: 1,
  },
  scrollContent: {
    padding: 20,
  },
  pageInfo: {
    marginBottom: 24,
  },
  pageTitle: {
    fontSize: 18,
    fontWeight: '700',
    marginBottom: 8,
    lineHeight: 24,
  },
  pageUrl: {
    fontSize: 13,
    fontFamily: 'monospace',
  },
  loadingContainer: {
    alignItems: 'center',
    paddingVertical: 48,
  },
  loadingText: {
    fontSize: 14,
    marginTop: 16,
  },
  section: {
    marginBottom: 24,
  },
  sectionHeader: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
    marginBottom: 12,
  },
  sectionTitle: {
    fontSize: 16,
    fontWeight: '700',
  },
  summaryText: {
    fontSize: 15,
    lineHeight: 24,
  },
  keyPoint: {
    flexDirection: 'row',
    padding: 16,
    borderRadius: 12,
    borderWidth: 1,
    marginBottom: 12,
    gap: 12,
  },
  bulletPoint: {
    width: 28,
    height: 28,
    borderRadius: 14,
    alignItems: 'center',
    justifyContent: 'center',
  },
  bulletText: {
    fontSize: 14,
    fontWeight: '700',
    color: '#fff',
  },
  pointText: {
    flex: 1,
    fontSize: 14,
    lineHeight: 20,
  },
  actions: {
    marginTop: 24,
    marginBottom: 32,
  },
  actionButton: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    padding: 16,
    borderRadius: 12,
    borderWidth: 2,
    gap: 8,
  },
  actionText: {
    fontSize: 16,
    fontWeight: '700',
  },
});
