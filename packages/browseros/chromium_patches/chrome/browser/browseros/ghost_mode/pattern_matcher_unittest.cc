diff --git a/chrome/browser/browseros/ghost_mode/pattern_matcher_unittest.cc b/chrome/browser/browseros/ghost_mode/pattern_matcher_unittest.cc
new file mode 100644
index 0000000000000..3c4d5e6f7a8b9
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/pattern_matcher_unittest.cc
@@ -0,0 +1,248 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/pattern_matcher.h"
+
+#include "testing/gtest/include/gtest/gtest.h"
+#include "url/gurl.h"
+
+namespace browseros::ghost_mode {
+
+class PatternMatcherTest : public testing::Test {
+ protected:
+  PatternMatcher matcher_;
+};
+
+// ============== Levenshtein Distance Tests ==============
+
+TEST_F(PatternMatcherTest, LevenshteinIdenticalStrings) {
+  EXPECT_EQ(LevenshteinDistance("hello", "hello"), 0);
+  EXPECT_EQ(LevenshteinDistance("", ""), 0);
+  EXPECT_EQ(LevenshteinDistance("test123", "test123"), 0);
+}
+
+TEST_F(PatternMatcherTest, LevenshteinEmptyString) {
+  EXPECT_EQ(LevenshteinDistance("hello", ""), 5);
+  EXPECT_EQ(LevenshteinDistance("", "world"), 5);
+}
+
+TEST_F(PatternMatcherTest, LevenshteinInsertion) {
+  EXPECT_EQ(LevenshteinDistance("cat", "cats"), 1);
+  EXPECT_EQ(LevenshteinDistance("button", "buttons"), 1);
+}
+
+TEST_F(PatternMatcherTest, LevenshteinDeletion) {
+  EXPECT_EQ(LevenshteinDistance("cats", "cat"), 1);
+  EXPECT_EQ(LevenshteinDistance("hello", "helo"), 1);
+}
+
+TEST_F(PatternMatcherTest, LevenshteinSubstitution) {
+  EXPECT_EQ(LevenshteinDistance("cat", "bat"), 1);
+  EXPECT_EQ(LevenshteinDistance("hello", "hallo"), 1);
+}
+
+TEST_F(PatternMatcherTest, LevenshteinComplex) {
+  EXPECT_EQ(LevenshteinDistance("kitten", "sitting"), 3);
+  EXPECT_EQ(LevenshteinDistance("Sunday", "Saturday"), 3);
+}
+
+// ============== URL Matching Tests ==============
+
+TEST_F(PatternMatcherTest, MatchURLExact) {
+  GURL recorded("https://example.com/login");
+  GURL current("https://example.com/login");
+  
+  double score = matcher_.MatchURL(recorded, current);
+  EXPECT_DOUBLE_EQ(score, 1.0);
+}
+
+TEST_F(PatternMatcherTest, MatchURLSameHostDifferentPath) {
+  GURL recorded("https://example.com/login");
+  GURL current("https://example.com/signup");
+  
+  double score = matcher_.MatchURL(recorded, current);
+  EXPECT_GT(score, 0.5);  // Same domain should have good score
+  EXPECT_LT(score, 1.0);
+}
+
+TEST_F(PatternMatcherTest, MatchURLDifferentHost) {
+  GURL recorded("https://example.com/login");
+  GURL current("https://other.com/login");
+  
+  double score = matcher_.MatchURL(recorded, current);
+  EXPECT_LT(score, 0.3);  // Different domain should have low score
+}
+
+TEST_F(PatternMatcherTest, MatchURLWithQueryParams) {
+  GURL recorded("https://example.com/search?q=test");
+  GURL current("https://example.com/search?q=other");
+  
+  double score = matcher_.MatchURL(recorded, current);
+  EXPECT_GT(score, 0.8);  // Same path, different params
+}
+
+TEST_F(PatternMatcherTest, MatchURLWithDynamicSegments) {
+  GURL recorded("https://example.com/user/123/profile");
+  GURL current("https://example.com/user/456/profile");
+  
+  double score = matcher_.MatchURL(recorded, current);
+  EXPECT_GT(score, 0.7);  // Dynamic segment difference
+}
+
+TEST_F(PatternMatcherTest, MatchURLSubdomain) {
+  GURL recorded("https://www.example.com/page");
+  GURL current("https://app.example.com/page");
+  
+  double score = matcher_.MatchURL(recorded, current);
+  EXPECT_GT(score, 0.5);  // Same root domain
+}
+
+// ============== Selector Matching Tests ==============
+
+TEST_F(PatternMatcherTest, MatchSelectorExact) {
+  double score = matcher_.MatchSelector("#submit-button", "#submit-button");
+  EXPECT_DOUBLE_EQ(score, 1.0);
+}
+
+TEST_F(PatternMatcherTest, MatchSelectorSimilar) {
+  double score = matcher_.MatchSelector("#submit-btn", "#submit-button");
+  EXPECT_GT(score, 0.5);
+}
+
+TEST_F(PatternMatcherTest, MatchSelectorClassVsId) {
+  double score = matcher_.MatchSelector("#button", ".button");
+  EXPECT_GT(score, 0.5);  // Same name, different type
+}
+
+TEST_F(PatternMatcherTest, MatchSelectorComplex) {
+  std::string recorded = "div.container > form > input[type='submit']";
+  std::string current = "div.wrapper > form > input[type='submit']";
+  
+  double score = matcher_.MatchSelector(recorded, current);
+  EXPECT_GT(score, 0.6);  // Similar structure
+}
+
+TEST_F(PatternMatcherTest, MatchSelectorDynamic) {
+  std::string recorded = "#item-12345";
+  std::string current = "#item-67890";
+  
+  double score = matcher_.MatchSelector(recorded, current);
+  EXPECT_GT(score, 0.6);  // Same pattern with different ID
+}
+
+TEST_F(PatternMatcherTest, MatchSelectorNthChild) {
+  std::string recorded = "ul > li:nth-child(3)";
+  std::string current = "ul > li:nth-child(5)";
+  
+  double score = matcher_.MatchSelector(recorded, current);
+  EXPECT_GT(score, 0.8);  // Same structure, different index
+}
+
+// ============== Selector List Matching Tests ==============
+
+TEST_F(PatternMatcherTest, MatchSelectorsFirstMatch) {
+  std::vector<std::string> recorded = {"#btn", ".button", "button[type=submit]"};
+  std::vector<std::string> current = {"#btn", ".other"};
+  
+  double score = matcher_.MatchSelectors(recorded, current);
+  EXPECT_DOUBLE_EQ(score, 1.0);  // Exact match on first
+}
+
+TEST_F(PatternMatcherTest, MatchSelectorsFallback) {
+  std::vector<std::string> recorded = {"#unique-id"};
+  std::vector<std::string> current = {".class", "button"};
+  
+  double score = matcher_.MatchSelectors(recorded, current);
+  EXPECT_LT(score, 0.5);  // No good match
+}
+
+TEST_F(PatternMatcherTest, MatchSelectorsEmpty) {
+  std::vector<std::string> recorded = {"#btn"};
+  std::vector<std::string> current = {};
+  
+  double score = matcher_.MatchSelectors(recorded, current);
+  EXPECT_DOUBLE_EQ(score, 0.0);
+}
+
+// ============== Action Matching Tests ==============
+
+TEST_F(PatternMatcherTest, MatchActionExact) {
+  RecordedAction recorded;
+  recorded.type = ActionType::kClick;
+  recorded.url = GURL("https://example.com/page");
+  recorded.selectors = {"#button"};
+  
+  RecordedAction current;
+  current.type = ActionType::kClick;
+  current.url = GURL("https://example.com/page");
+  current.selectors = {"#button"};
+  
+  double score = matcher_.MatchAction(recorded, current);
+  EXPECT_DOUBLE_EQ(score, 1.0);
+}
+
+TEST_F(PatternMatcherTest, MatchActionDifferentType) {
+  RecordedAction recorded;
+  recorded.type = ActionType::kClick;
+  recorded.url = GURL("https://example.com");
+  recorded.selectors = {"#btn"};
+  
+  RecordedAction current;
+  current.type = ActionType::kType;
+  current.url = GURL("https://example.com");
+  current.selectors = {"#btn"};
+  
+  double score = matcher_.MatchAction(recorded, current);
+  EXPECT_DOUBLE_EQ(score, 0.0);  // Type mismatch is critical
+}
+
+TEST_F(PatternMatcherTest, MatchActionSimilarURL) {
+  RecordedAction recorded;
+  recorded.type = ActionType::kClick;
+  recorded.url = GURL("https://example.com/products/123");
+  recorded.selectors = {"#add-to-cart"};
+  
+  RecordedAction current;
+  current.type = ActionType::kClick;
+  current.url = GURL("https://example.com/products/456");
+  current.selectors = {"#add-to-cart"};
+  
+  double score = matcher_.MatchAction(recorded, current);
+  EXPECT_GT(score, 0.8);
+}
+
+// ============== URL Pattern Extraction Tests ==============
+
+TEST_F(PatternMatcherTest, ExtractURLPattern) {
+  GURL url("https://example.com/user/123/posts/456");
+  std::string pattern = ExtractURLPattern(url);
+  
+  // Pattern should replace numeric IDs with placeholders
+  EXPECT_TRUE(pattern.find("{id}") != std::string::npos ||
+              pattern.find("*") != std::string::npos);
+}
+
+TEST_F(PatternMatcherTest, ExtractSelectorPattern) {
+  std::string selector = "#item-12345";
+  std::string pattern = ExtractSelectorPattern(selector);
+  
+  // Should extract pattern without specific ID
+  EXPECT_NE(pattern, selector);
+}
+
+// ============== Threshold Tests ==============
+
+TEST_F(PatternMatcherTest, IsGoodMatchRespectThreshold) {
+  EXPECT_TRUE(IsGoodMatch(0.9, 0.8));
+  EXPECT_TRUE(IsGoodMatch(0.8, 0.8));
+  EXPECT_FALSE(IsGoodMatch(0.79, 0.8));
+  EXPECT_FALSE(IsGoodMatch(0.5, 0.8));
+}
+
+TEST_F(PatternMatcherTest, IsGoodMatchDefaultThreshold) {
+  // Default threshold should be around 0.7-0.8
+  EXPECT_TRUE(IsGoodMatch(0.85));
+  EXPECT_FALSE(IsGoodMatch(0.5));
+}
+
+}  // namespace browseros::ghost_mode
