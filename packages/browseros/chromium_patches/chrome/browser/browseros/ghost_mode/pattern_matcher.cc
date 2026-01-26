diff --git a/chrome/browser/browseros/ghost_mode/pattern_matcher.cc b/chrome/browser/browseros/ghost_mode/pattern_matcher.cc
new file mode 100644
index 0000000000000..e5f6a7b8c9d0e
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/pattern_matcher.cc
@@ -0,0 +1,305 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/pattern_matcher.h"
+
+#include <algorithm>
+#include <regex>
+
+#include "base/no_destructor.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/strings/string_split.h"
+#include "base/strings/string_util.h"
+
+namespace browseros::ghost_mode {
+
+namespace {
+
+// Regex patterns for identifying dynamic content
+const std::regex kUuidPattern(
+    "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
+    std::regex::icase);
+const std::regex kNumericIdPattern("^\\d+$");
+const std::regex kAlphanumericIdPattern("^[a-zA-Z0-9]{20,}$");
+const std::regex kDynamicClassPattern(
+    "\\b(css-[a-z0-9]+|sc-[a-zA-Z0-9]+|_[a-zA-Z0-9]{5,})\\b");
+
+}  // namespace
+
+PatternMatcher::PatternMatcher() = default;
+PatternMatcher::~PatternMatcher() = default;
+
+bool PatternMatcher::UrlsMatch(const GURL& url1, const GURL& url2) const {
+  return CalculateUrlSimilarity(url1, url2) >= similarity_threshold_;
+}
+
+std::string PatternMatcher::GenerateUrlPattern(const GURL& url) const {
+  if (!url.is_valid()) {
+    return "";
+  }
+  
+  std::string pattern = url.host();
+  
+  std::vector<std::string> segments = SplitPath(url.path());
+  
+  for (const auto& segment : segments) {
+    if (segment.empty()) {
+      continue;
+    }
+    
+    pattern += "/";
+    
+    if (IsLikelyId(segment)) {
+      pattern += "*";  // Wildcard for IDs
+    } else {
+      pattern += segment;
+    }
+  }
+  
+  return pattern;
+}
+
+bool PatternMatcher::UrlMatchesPattern(const GURL& url,
+                                         const std::string& pattern) const {
+  if (!url.is_valid() || pattern.empty()) {
+    return false;
+  }
+  
+  // Split pattern into host and path parts
+  size_t first_slash = pattern.find('/');
+  std::string pattern_host = (first_slash != std::string::npos)
+                                  ? pattern.substr(0, first_slash)
+                                  : pattern;
+  std::string pattern_path = (first_slash != std::string::npos)
+                                  ? pattern.substr(first_slash)
+                                  : "";
+  
+  // Check host match
+  if (url.host() != pattern_host) {
+    return false;
+  }
+  
+  // Split paths into segments
+  std::vector<std::string> url_segments = SplitPath(url.path());
+  std::vector<std::string> pattern_segments = SplitPath(pattern_path);
+  
+  if (url_segments.size() != pattern_segments.size()) {
+    return false;
+  }
+  
+  // Compare each segment
+  for (size_t i = 0; i < url_segments.size(); ++i) {
+    if (pattern_segments[i] == "*") {
+      continue;  // Wildcard matches anything
+    }
+    if (url_segments[i] != pattern_segments[i]) {
+      return false;
+    }
+  }
+  
+  return true;
+}
+
+double PatternMatcher::CalculateUrlSimilarity(const GURL& url1,
+                                               const GURL& url2) const {
+  if (!url1.is_valid() || !url2.is_valid()) {
+    return 0.0;
+  }
+  
+  // Same host is required
+  if (url1.host() != url2.host()) {
+    return 0.0;
+  }
+  
+  // Same scheme preferred
+  double scheme_score = (url1.scheme() == url2.scheme()) ? 1.0 : 0.8;
+  
+  // Compare path segments
+  std::vector<std::string> segments1 = SplitPath(url1.path());
+  std::vector<std::string> segments2 = SplitPath(url2.path());
+  
+  if (segments1.empty() && segments2.empty()) {
+    return scheme_score;
+  }
+  
+  // Calculate path similarity
+  size_t max_len = std::max(segments1.size(), segments2.size());
+  size_t min_len = std::min(segments1.size(), segments2.size());
+  
+  int matches = 0;
+  for (size_t i = 0; i < min_len; ++i) {
+    if (segments1[i] == segments2[i]) {
+      matches += 2;  // Exact match
+    } else if (IsLikelyId(segments1[i]) && IsLikelyId(segments2[i])) {
+      matches += 1;  // Both are IDs (likely same slot)
+    }
+  }
+  
+  double path_score = static_cast<double>(matches) / (2 * max_len);
+  
+  return (scheme_score * 0.1) + (path_score * 0.9);
+}
+
+bool PatternMatcher::SelectorsMatch(const std::string& sel1,
+                                      const std::string& sel2) const {
+  return CalculateSelectorSimilarity(sel1, sel2) >= similarity_threshold_;
+}
+
+std::string PatternMatcher::FindBestMatchingSelector(
+    const std::string& target,
+    const std::vector<std::string>& candidates) const {
+  std::string best;
+  double best_score = 0.0;
+  
+  for (const auto& candidate : candidates) {
+    double score = CalculateSelectorSimilarity(target, candidate);
+    if (score > best_score) {
+      best_score = score;
+      best = candidate;
+    }
+  }
+  
+  return (best_score >= similarity_threshold_) ? best : "";
+}
+
+double PatternMatcher::CalculateSelectorSimilarity(
+    const std::string& sel1,
+    const std::string& sel2) const {
+  if (sel1.empty() || sel2.empty()) {
+    return 0.0;
+  }
+  
+  // Exact match
+  if (sel1 == sel2) {
+    return 1.0;
+  }
+  
+  // Normalize both selectors
+  std::string norm1 = GenerateSelectorPattern(sel1);
+  std::string norm2 = GenerateSelectorPattern(sel2);
+  
+  if (norm1 == norm2) {
+    return 0.95;  // Same after normalization
+  }
+  
+  // Parse into components
+  auto comps1 = ParseSelector(sel1);
+  auto comps2 = ParseSelector(sel2);
+  
+  if (comps1.empty() || comps2.empty()) {
+    return 0.0;
+  }
+  
+  // Compare components
+  int matches = 0;
+  int total = static_cast<int>(std::max(comps1.size(), comps2.size()));
+  
+  for (const auto& c1 : comps1) {
+    for (const auto& c2 : comps2) {
+      if (c1.type == c2.type && c1.value == c2.value) {
+        matches++;
+        break;
+      }
+    }
+  }
+  
+  double component_score = static_cast<double>(matches) / total;
+  
+  // Calculate string similarity as fallback
+  int max_len = static_cast<int>(std::max(sel1.length(), sel2.length()));
+  int edit_dist = LevenshteinDistance(sel1, sel2);
+  double string_score = 1.0 - (static_cast<double>(edit_dist) / max_len);
+  
+  return (component_score * 0.7) + (string_score * 0.3);
+}
+
+std::string PatternMatcher::GenerateSelectorPattern(
+    const std::string& selector) const {
+  std::string result = selector;
+  
+  // Remove dynamic class names (CSS-in-JS generated)
+  result = std::regex_replace(result, kDynamicClassPattern, "*");
+  
+  // Normalize whitespace
+  result = base::CollapseWhitespaceASCII(result, true);
+  
+  return result;
+}
+
+std::vector<std::string> PatternMatcher::SplitPath(
+    const std::string& path) const {
+  std::vector<std::string> segments;
+  
+  for (const auto& segment :
+       base::SplitString(path, "/", base::KEEP_WHITESPACE,
+                         base::SPLIT_WANT_NONEMPTY)) {
+    segments.push_back(segment);
+  }
+  
+  return segments;
+}
+
+bool PatternMatcher::IsLikelyId(const std::string& segment) const {
+  if (segment.empty()) {
+    return false;
+  }
+  
+  // Check if it's a UUID
+  if (std::regex_match(segment, kUuidPattern)) {
+    return true;
+  }
+  
+  // Check if it's numeric
+  if (std::regex_match(segment, kNumericIdPattern)) {
+    return true;
+  }
+  
+  // Check if it's a long alphanumeric string (likely generated ID)
+  if (std::regex_match(segment, kAlphanumericIdPattern)) {
+    return true;
+  }
+  
+  return false;
+}
+
+std::vector<PatternMatcher::SelectorComponent> PatternMatcher::ParseSelector(
+    const std::string& selector) const {
+  std::vector<SelectorComponent> components;
+  
+  // Simple parsing - look for common patterns
+  // This is a simplified parser; real CSS selector parsing is more complex
+  
+  // ID selectors (#id)
+  std::regex id_regex("#([a-zA-Z][a-zA-Z0-9_-]*)");
+  std::smatch id_match;
+  std::string temp = selector;
+  while (std::regex_search(temp, id_match, id_regex)) {
+    components.push_back({"id", id_match[1].str(), 100});
+    temp = id_match.suffix();
+  }
+  
+  // Class selectors (.class)
+  std::regex class_regex("\\.([a-zA-Z][a-zA-Z0-9_-]*)");
+  std::smatch class_match;
+  temp = selector;
+  while (std::regex_search(temp, class_match, class_regex)) {
+    components.push_back({"class", class_match[1].str(), 10});
+    temp = class_match.suffix();
+  }
+  
+  // Attribute selectors ([attr=value])
+  std::regex attr_regex("\\[([a-zA-Z-]+)=['\"]?([^'\"\\]]+)['\"]?\\]");
+  std::smatch attr_match;
+  temp = selector;
+  while (std::regex_search(temp, attr_match, attr_regex)) {
+    std::string attr_name = attr_match[1].str();
+    int specificity = (attr_name == "data-testid" || attr_name == "data-test")
+                          ? 90
+                          : 40;
+    components.push_back({"attr", attr_name + "=" + attr_match[2].str(),
+                          specificity});
+    temp = attr_match.suffix();
+  }
+  
+  return components;
+}
+
+int PatternMatcher::LevenshteinDistance(const std::string& s1,
+                                         const std::string& s2) const {
+  size_t len1 = s1.length();
+  size_t len2 = s2.length();
+  
+  std::vector<std::vector<int>> dp(len1 + 1, std::vector<int>(len2 + 1));
+  
+  for (size_t i = 0; i <= len1; ++i) {
+    dp[i][0] = static_cast<int>(i);
+  }
+  for (size_t j = 0; j <= len2; ++j) {
+    dp[0][j] = static_cast<int>(j);
+  }
+  
+  for (size_t i = 1; i <= len1; ++i) {
+    for (size_t j = 1; j <= len2; ++j) {
+      int cost = (s1[i - 1] == s2[j - 1]) ? 0 : 1;
+      dp[i][j] = std::min({dp[i - 1][j] + 1,       // deletion
+                           dp[i][j - 1] + 1,       // insertion
+                           dp[i - 1][j - 1] + cost // substitution
+                          });
+    }
+  }
+  
+  return dp[len1][len2];
+}
+
+// Singleton
+PatternMatcher& GetPatternMatcher() {
+  static base::NoDestructor<PatternMatcher> instance;
+  return *instance;
+}
+
+// Convenience functions
+bool UrlsLikelySamePage(const GURL& url1, const GURL& url2) {
+  return GetPatternMatcher().UrlsMatch(url1, url2);
+}
+
+bool SelectorsLikelySameElement(const std::string& sel1,
+                                 const std::string& sel2) {
+  return GetPatternMatcher().SelectorsMatch(sel1, sel2);
+}
+
+}  // namespace browseros::ghost_mode
