import React, { useEffect, useRef } from 'react';
import { View, StyleSheet, useColorScheme, Alert } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { Colors } from '@/constants/Colors';
import { useBrowserStore } from '@/store/browser-store';
import { WebViewBrowser, WebViewBrowserRef } from '@/components/browser/WebViewBrowser';
import { AddressBar } from '@/components/browser/AddressBar';

export default function BrowserScreen() {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;
  const webViewRef = useRef<WebViewBrowserRef>(null);

  const {
    tabs,
    activeTabId,
    addTab,
    updateTab,
    setActiveTab,
    bookmarks,
    addBookmark,
    removeBookmark,
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
    webViewRef.current?.goBack();
  };

  const handleGoForward = () => {
    webViewRef.current?.goForward();
  };

  const handleReload = () => {
    webViewRef.current?.reload();
  };

  const handleStop = () => {
    webViewRef.current?.stopLoading();
  };

  const handleBookmark = () => {
    if (!activeTab) return;

    const isBookmarked = bookmarks.some((b) => b.url === activeTab.url);

    if (isBookmarked) {
      // Remove bookmark
      const bookmark = bookmarks.find((b) => b.url === activeTab.url);
      if (bookmark) {
        removeBookmark(bookmark.id);
        Alert.alert('Bookmark Removed', `Removed "${activeTab.title}" from bookmarks`);
      }
    } else {
      // Add bookmark
      addBookmark(activeTab.url, activeTab.title, activeTab.favicon);
      Alert.alert('Bookmark Added', `Added "${activeTab.title}" to bookmarks`);
    }
  };

  // Check if current URL is bookmarked
  const isBookmarked = activeTab
    ? bookmarks.some((b) => b.url === activeTab.url)
    : false;

  if (!activeTab) {
    return (
      <SafeAreaView
        style={[styles.container, { backgroundColor: colors.background }]}
        edges={['top', 'bottom']}
      />
    );
  }

  return (
    <SafeAreaView
      style={[styles.container, { backgroundColor: colors.background }]}
      edges={['bottom']}
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
        onReload={handleReload}
        onStop={handleStop}
        onBookmark={handleBookmark}
        isBookmarked={isBookmarked}
      />

      {/* WebView Browser */}
      <View style={styles.webviewContainer}>
        <WebViewBrowser
          ref={webViewRef}
          tabId={activeTab.id}
          url={activeTab.url}
          isIncognito={activeTab.isIncognito}
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
