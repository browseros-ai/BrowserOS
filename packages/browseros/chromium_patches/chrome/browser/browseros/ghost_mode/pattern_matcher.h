diff --git a/chrome/browser/browseros/ghost_mode/pattern_matcher.h b/chrome/browser/browseros/ghost_mode/pattern_matcher.h
new file mode 100644
index 0000000000000..d4e5f6a7b8c9d
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/pattern_matcher.h
@@ -0,0 +1,99 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_GHOST_MODE_PATTERN_MATCHER_H_
+#define CHROME_BROWSER_BROWSEROS_GHOST_MODE_PATTERN_MATCHER_H_
+
+#include <string>
+#include <vector>
+
+#include "url/gurl.h"
+
+namespace browseros::ghost_mode {
+
+// PatternMatcher provides fuzzy matching for URLs and CSS selectors.
+// This allows Ghost Mode to recognize patterns even when page structure
+// varies slightly (e.g., dynamic IDs, slightly different paths).
+class PatternMatcher {
+ public:
+  PatternMatcher();
+  ~PatternMatcher();
+
+  // URL Matching
+  
+  // Check if two URLs match according to a pattern
+  // Ignores query parameters, fragments, and allows wildcards in path
+  bool UrlsMatch(const GURL& url1, const GURL& url2) const;
+  
+  // Generate a URL pattern from a URL
+  // e.g., "example.com/users/123/posts" -> "example.com/users/*/posts"
+  std::string GenerateUrlPattern(const GURL& url) const;
+  
+  // Check if a URL matches a pattern string
+  bool UrlMatchesPattern(const GURL& url, const std::string& pattern) const;
+  
+  // Calculate URL similarity (0.0 - 1.0)
+  double CalculateUrlSimilarity(const GURL& url1, const GURL& url2) const;
+
+  // Selector Matching
+  
+  // Check if two CSS selectors likely target the same element
+  bool SelectorsMatch(const std::string& sel1, const std::string& sel2) const;
+  
+  // Find the best matching selector from a list
+  std::string FindBestMatchingSelector(
+      const std::string& target,
+      const std::vector<std::string>& candidates) const;
+  
+  // Calculate selector similarity (0.0 - 1.0)
+  double CalculateSelectorSimilarity(const std::string& sel1,
+                                      const std::string& sel2) const;
+  
+  // Generate a stable selector pattern (normalizes dynamic parts)
+  std::string GenerateSelectorPattern(const std::string& selector) const;
+
+  // Configuration
+  
+  // Minimum similarity threshold for matching (default: 0.8)
+  void SetSimilarityThreshold(double threshold) {
+    similarity_threshold_ = threshold;
+  }
+  
+  double GetSimilarityThreshold() const { return similarity_threshold_; }
+
+ private:
+  // Helper to split URL path into segments
+  std::vector<std::string> SplitPath(const std::string& path) const;
+  
+  // Helper to check if a path segment looks like an ID (numeric, UUID, etc.)
+  bool IsLikelyId(const std::string& segment) const;
+  
+  // Helper to parse CSS selector into components
+  struct SelectorComponent {
+    std::string type;      // "id", "class", "tag", "attr", "nth", "pseudo"
+    std::string value;
+    int specificity = 0;   // Higher = more specific
+  };
+  std::vector<SelectorComponent> ParseSelector(
+      const std::string& selector) const;
+  
+  // Calculate Levenshtein distance between strings
+  int LevenshteinDistance(const std::string& s1, const std::string& s2) const;
+  
+  // Configuration
+  double similarity_threshold_ = 0.8;
+};
+
+// Singleton accessor
+PatternMatcher& GetPatternMatcher();
+
+// Convenience functions
+
+// Quick check if two URLs are likely the same page (different instances)
+bool UrlsLikelySamePage(const GURL& url1, const GURL& url2);
+
+// Quick check if two selectors target similar elements
+bool SelectorsLikelySameElement(const std::string& sel1,
+                                 const std::string& sel2);
+
+}  // namespace browseros::ghost_mode
+
+#endif  // CHROME_BROWSER_BROWSEROS_GHOST_MODE_PATTERN_MATCHER_H_
