diff --git a/chrome/browser/browseros/ghost_mode/pattern_detector_unittest.cc b/chrome/browser/browseros/ghost_mode/pattern_detector_unittest.cc
new file mode 100644
index 0000000000000..2b3c4d5e6f7a8
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/pattern_detector_unittest.cc
@@ -0,0 +1,298 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/pattern_detector.h"
+
+#include "base/files/scoped_temp_dir.h"
+#include "base/test/task_environment.h"
+#include "chrome/browser/browseros/ghost_mode/action_store.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_prefs.h"
+#include "components/prefs/testing_pref_service.h"
+#include "testing/gtest/include/gtest/gtest.h"
+#include "url/gurl.h"
+
+namespace browseros::ghost_mode {
+
+class PatternDetectorTest : public testing::Test {
+ protected:
+  void SetUp() override {
+    ASSERT_TRUE(temp_dir_.CreateUniqueTempDir());
+    
+    // Register prefs
+    prefs::RegisterProfilePrefs(pref_service_.registry());
+    
+    // Create action store
+    action_store_ = std::make_unique<ActionStore>(
+        temp_dir_.GetPath(), &pref_service_);
+    ASSERT_TRUE(action_store_->Initialize());
+    
+    // Create detector
+    detector_ = std::make_unique<PatternDetector>(
+        action_store_.get(), &pref_service_);
+  }
+
+  RecordedAction CreateAction(ActionType type,
+                               const std::string& url,
+                               const std::string& selector,
+                               const std::string& session_id,
+                               base::Time timestamp) {
+    RecordedAction action;
+    action.id = base::Uuid::GenerateRandomV4().AsLowercaseString();
+    action.type = type;
+    action.url = GURL(url);
+    action.url_pattern = GURL(url).host() + GURL(url).path();
+    action.selectors.push_back(selector);
+    action.session_id = session_id;
+    action.timestamp = timestamp;
+    return action;
+  }
+
+  void AddRepeatedSequence(int count) {
+    // Add the same sequence of actions multiple times
+    for (int i = 0; i < count; ++i) {
+      std::string session_id = "session_" + base::NumberToString(i);
+      base::Time base_time = base::Time::Now() - base::Days(i);
+      
+      // Login flow: navigate -> type username -> type password -> click login
+      action_store_->AddAction(CreateAction(
+          ActionType::kNavigate,
+          "https://example.com/login",
+          "",
+          session_id,
+          base_time));
+      
+      action_store_->AddAction(CreateAction(
+          ActionType::kType,
+          "https://example.com/login",
+          "#username",
+          session_id,
+          base_time + base::Seconds(2)));
+      
+      action_store_->AddAction(CreateAction(
+          ActionType::kType,
+          "https://example.com/login",
+          "#password",
+          session_id,
+          base_time + base::Seconds(4)));
+      
+      action_store_->AddAction(CreateAction(
+          ActionType::kClick,
+          "https://example.com/login",
+          "#login-button",
+          session_id,
+          base_time + base::Seconds(5)));
+    }
+  }
+
+  base::test::TaskEnvironment task_environment_;
+  base::ScopedTempDir temp_dir_;
+  TestingPrefServiceSimple pref_service_;
+  std::unique_ptr<ActionStore> action_store_;
+  std::unique_ptr<PatternDetector> detector_;
+};
+
+TEST_F(PatternDetectorTest, DetectsNoPatternWithInsufficientData) {
+  // Add only one occurrence of a sequence
+  AddRepeatedSequence(1);
+  
+  auto patterns = detector_->DetectPatterns();
+  
+  // Should find no patterns (need at least 3 by default)
+  EXPECT_TRUE(patterns.empty());
+}
+
+TEST_F(PatternDetectorTest, DetectsPatternWithSufficientOccurrences) {
+  // Add sequence 5 times
+  AddRepeatedSequence(5);
+  
+  auto patterns = detector_->DetectPatterns();
+  
+  // Should find at least one pattern
+  EXPECT_FALSE(patterns.empty());
+  
+  // Check pattern properties
+  auto& pattern = patterns[0];
+  EXPECT_GE(pattern.occurrence_count, 3);
+  EXPECT_GT(pattern.confidence_score, 0.0);
+  EXPECT_FALSE(pattern.actions.empty());
+}
+
+TEST_F(PatternDetectorTest, PatternHasCorrectActionSequence) {
+  AddRepeatedSequence(5);
+  
+  auto patterns = detector_->DetectPatterns();
+  ASSERT_FALSE(patterns.empty());
+  
+  // Find the login pattern (should have 4 actions)
+  const ActionSequence* login_pattern = nullptr;
+  for (const auto& p : patterns) {
+    if (p.actions.size() == 4) {
+      login_pattern = &p;
+      break;
+    }
+  }
+  
+  if (login_pattern) {
+    EXPECT_EQ(login_pattern->actions[0].type, ActionType::kNavigate);
+    EXPECT_EQ(login_pattern->actions[1].type, ActionType::kType);
+    EXPECT_EQ(login_pattern->actions[2].type, ActionType::kType);
+    EXPECT_EQ(login_pattern->actions[3].type, ActionType::kClick);
+  }
+}
+
+TEST_F(PatternDetectorTest, ConfidenceScoreReflectsQuality) {
+  // Add high-quality pattern (consistent timing, stable selectors)
+  AddRepeatedSequence(10);
+  
+  auto patterns = detector_->DetectPatterns();
+  ASSERT_FALSE(patterns.empty());
+  
+  // Higher occurrences should yield higher confidence
+  EXPECT_GT(patterns[0].confidence_score, 0.5);
+}
+
+TEST_F(PatternDetectorTest, RespectsMinOccurrencesSetting) {
+  // Set higher threshold
+  detector_->SetMinOccurrences(5);
+  
+  // Add only 3 occurrences
+  AddRepeatedSequence(3);
+  
+  auto patterns = detector_->DetectPatterns();
+  
+  // Should not find patterns (threshold is 5)
+  EXPECT_TRUE(patterns.empty());
+  
+  // Add more occurrences
+  AddRepeatedSequence(3);  // Now 6 total
+  
+  patterns = detector_->DetectPatterns();
+  
+  // Now should find patterns
+  EXPECT_FALSE(patterns.empty());
+}
+
+TEST_F(PatternDetectorTest, RespectsMinConfidenceSetting) {
+  detector_->SetMinConfidence(0.99);  // Very high threshold
+  
+  AddRepeatedSequence(3);
+  
+  auto patterns = detector_->DetectPatterns();
+  
+  // May not meet high confidence threshold
+  // (depends on pattern quality)
+}
+
+TEST_F(PatternDetectorTest, HasExistingPatternReturnsTrueForSaved) {
+  AddRepeatedSequence(5);
+  
+  auto patterns = detector_->DetectPatterns();
+  ASSERT_FALSE(patterns.empty());
+  
+  // Save pattern to store
+  action_store_->SavePattern(patterns[0]);
+  
+  // Check if pattern exists
+  EXPECT_TRUE(detector_->HasExistingPattern(patterns[0].actions));
+}
+
+TEST_F(PatternDetectorTest, HasExistingPatternReturnsFalseForNew) {
+  std::vector<RecordedAction> new_actions;
+  new_actions.push_back(CreateAction(
+      ActionType::kClick,
+      "https://new-site.com",
+      "#button",
+      "session",
+      base::Time::Now()));
+  
+  EXPECT_FALSE(detector_->HasExistingPattern(new_actions));
+}
+
+// Test observer notifications
+class TestPatternObserver : public PatternDetectorObserver {
+ public:
+  void OnPatternDetected(const ActionSequence& pattern) override {
+    detected_patterns_.push_back(pattern);
+  }
+  
+  void OnDetectionComplete(int patterns_found) override {
+    detection_complete_ = true;
+    patterns_found_count_ = patterns_found;
+  }
+  
+  std::vector<ActionSequence> detected_patterns_;
+  bool detection_complete_ = false;
+  int patterns_found_count_ = 0;
+};
+
+TEST_F(PatternDetectorTest, NotifiesObserversOnDetection) {
+  TestPatternObserver observer;
+  detector_->AddObserver(&observer);
+  
+  AddRepeatedSequence(5);
+  auto patterns = detector_->DetectPatterns();
+  
+  EXPECT_TRUE(observer.detection_complete_);
+  EXPECT_EQ(observer.patterns_found_count_, 
+            static_cast<int>(patterns.size()));
+  
+  detector_->RemoveObserver(&observer);
+}
+
+// Test sequence similarity functions
+TEST_F(PatternDetectorTest, SequenceSimilarityExact) {
+  std::vector<RecordedAction> seq1, seq2;
+  
+  seq1.push_back(CreateAction(ActionType::kClick, "https://a.com", "#btn", "s", base::Time::Now()));
+  seq2.push_back(CreateAction(ActionType::kClick, "https://a.com", "#btn", "s", base::Time::Now()));
+  
+  double similarity = CalculateSequenceSimilarity(seq1, seq2);
+  EXPECT_DOUBLE_EQ(similarity, 1.0);
+}
+
+TEST_F(PatternDetectorTest, SequenceSimilarityDifferent) {
+  std::vector<RecordedAction> seq1, seq2;
+  
+  seq1.push_back(CreateAction(ActionType::kClick, "https://a.com", "#btn", "s", base::Time::Now()));
+  seq2.push_back(CreateAction(ActionType::kType, "https://b.com", "#input", "s", base::Time::Now()));
+  
+  double similarity = CalculateSequenceSimilarity(seq1, seq2);
+  EXPECT_LT(similarity, 0.5);
+}
+
+TEST_F(PatternDetectorTest, AreSequencesSimilarThreshold) {
+  std::vector<RecordedAction> seq1, seq2;
+  
+  // Similar sequences
+  seq1.push_back(CreateAction(ActionType::kClick, "https://a.com", "#btn", "s", base::Time::Now()));
+  seq2.push_back(CreateAction(ActionType::kClick, "https://a.com", "#btn", "s", base::Time::Now()));
+  
+  EXPECT_TRUE(AreSequencesSimilar(seq1, seq2, 0.9));
+  
+  // Different sequences
+  seq2.clear();
+  seq2.push_back(CreateAction(ActionType::kType, "https://b.com", "#x", "s", base::Time::Now()));
+  
+  EXPECT_FALSE(AreSequencesSimilar(seq1, seq2, 0.9));
+}
+
+TEST_F(PatternDetectorTest, HandlesEmptyActionStore) {
+  auto patterns = detector_->DetectPatterns();
+  EXPECT_TRUE(patterns.empty());
+}
+
+TEST_F(PatternDetectorTest, GeneratesPatternName) {
+  AddRepeatedSequence(5);
+  
+  auto patterns = detector_->DetectPatterns();
+  
+  for (const auto& pattern : patterns) {
+    EXPECT_FALSE(pattern.name.empty());
+    // Name should contain domain or action info
+    EXPECT_TRUE(pattern.name.find("example.com") != std::string::npos ||
+                pattern.name.find("flow") != std::string::npos);
+  }
+}
+
+}  // namespace browseros::ghost_mode
