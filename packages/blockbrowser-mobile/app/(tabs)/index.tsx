import React, { useEffect } from 'react';
import { View, StyleSheet, useColorScheme } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { Colors } from '@/constants/Colors';
import { useBrowserStore } from '@/store/browser-store';
import { WebViewBrowser } from '@/components/browser/WebViewBrowser';
import { AddressBar } from '@/components/browser/AddressBar';

export default function BrowserScreen() {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;

  const {
    tabs,
    activeTabId,
    addTab,
    updateTab,
    setActiveTab,
    addToHistory,
  } = useBrowserStore();

  const activeTab = tabs.find((tab) => tab.id === activeTabId);

  // Create initial tab if none exist
  useEffect(() => {
    if (tabs.length === 0) {
      const newTabId = addTab('https://www.google.com');
      setActiveTab(newTabId);
    }
  }, []);

  const handleNavigate = (url: string) => {
    if (activeTabId) {
      updateTab(activeTabId, { url });
    }
  };

  const handleGoBack = () => {
    // WebView ref will handle this
  };

  const handleGoForward = () => {
    // WebView ref will handle this
  };

  const handleRefresh = () => {
    // WebView ref will handle this
  };

  const handleLoadStart = () => {
    if (activeTabId) {
      updateTab(activeTabId, { isLoading: true, progress: 0 });
    }
  };

  const handleLoadProgress = (progress: number) => {
    if (activeTabId) {
      updateTab(activeTabId, { progress });
    }
  };

  const handleLoadEnd = () => {
    if (activeTabId) {
      updateTab(activeTabId, { isLoading: false, progress: 1 });
    }
  };

  const handlePageInfoUpdate = (info: {
    title: string;
    url: string;
    favicon?: string;
    canGoBack: boolean;
    canGoForward: boolean;
  }) => {
    if (activeTabId) {
      updateTab(activeTabId, info);
      addToHistory(info.url, info.title);
    }
  };

  if (!activeTab) {
    return (
      <SafeAreaView
        style={[styles.container, { backgroundColor: colors.background }]}
        edges={['top']}
      />
    );
  }

  return (
    <SafeAreaView
      style={[styles.container, { backgroundColor: colors.background }]}
      edges={['top']}
    >
      {/* Address Bar */}
      <AddressBar
        url={activeTab.url}
        isLoading={activeTab.isLoading}
        progress={activeTab.progress}
        canGoBack={activeTab.canGoBack}
        canGoForward={activeTab.canGoForward}
        onNavigate={handleNavigate}
        onGoBack={handleGoBack}
        onGoForward={handleGoForward}
        onRefresh={handleRefresh}
      />

      {/* WebView Browser */}
      <View style={styles.webviewContainer}>
        <WebViewBrowser
          url={activeTab.url}
          onLoadStart={handleLoadStart}
          onLoadProgress={handleLoadProgress}
          onLoadEnd={handleLoadEnd}
          onPageInfoUpdate={handlePageInfoUpdate}
        />
      </View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  webviewContainer: {
    flex: 1,
  },
});
