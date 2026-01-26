diff --git a/chrome/browser/browseros/ghost_mode/pattern_detector.h b/chrome/browser/browseros/ghost_mode/pattern_detector.h
new file mode 100644
index 0000000000000..0c1d2e3f4a5b6
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/pattern_detector.h
@@ -0,0 +1,145 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_GHOST_MODE_PATTERN_DETECTOR_H_
+#define CHROME_BROWSER_BROWSEROS_GHOST_MODE_PATTERN_DETECTOR_H_
+
+#include <memory>
+#include <string>
+#include <unordered_map>
+#include <vector>
+
+#include "base/memory/raw_ptr.h"
+#include "base/memory/weak_ptr.h"
+#include "base/observer_list.h"
+#include "base/time/time.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_types.h"
+
+class PrefService;
+
+namespace browseros::ghost_mode {
+
+class ActionStore;
+
+// Observer for pattern detection events
+class PatternDetectorObserver {
+ public:
+  virtual ~PatternDetectorObserver() = default;
+  
+  // Called when a new pattern is detected that meets the threshold
+  virtual void OnPatternDetected(const ActionSequence& pattern) {}
+  
+  // Called when detection scan completes
+  virtual void OnDetectionComplete(int patterns_found) {}
+};
+
+// PatternDetector analyzes recorded actions to find repeated sequences
+// that could be automated.
+//
+// Algorithm overview:
+// 1. Group actions by session
+// 2. Extract subsequences of length 3-20
+// 3. Normalize subsequences (strip variable data)
+// 4. Hash and count occurrences
+// 5. Return sequences meeting threshold (default: 3 occurrences, 0.8 confidence)
+//
+// The detector runs periodically in the background or on-demand.
+class PatternDetector {
+ public:
+  PatternDetector(ActionStore* action_store, PrefService* pref_service);
+  ~PatternDetector();
+
+  // Run pattern detection on all stored actions
+  // Returns patterns meeting the configured thresholds
+  std::vector<ActionSequence> DetectPatterns();
+
+  // Run detection asynchronously
+  void DetectPatternsAsync(
+      base::OnceCallback<void(std::vector<ActionSequence>)> callback);
+
+  // Check if a specific action sequence already exists as a pattern
+  bool HasExistingPattern(const std::vector<RecordedAction>& actions);
+
+  // Observer management
+  void AddObserver(PatternDetectorObserver* observer);
+  void RemoveObserver(PatternDetectorObserver* observer);
+
+  // Configuration
+  void SetMinSequenceLength(int length) { min_sequence_length_ = length; }
+  void SetMaxSequenceLength(int length) { max_sequence_length_ = length; }
+  void SetMinOccurrences(int count) { min_occurrences_ = count; }
+  void SetMinConfidence(double confidence) { min_confidence_ = confidence; }
+
+ private:
+  // Internal structure for tracking candidate patterns
+  struct CandidatePattern {
+    std::string hash;
+    std::vector<RecordedAction> actions;
+    std::vector<base::Time> occurrence_times;
+    double confidence_score = 0.0;
+  };
+
+  // Group actions into sessions
+  std::unordered_map<std::string, std::vector<RecordedAction>> 
+      GroupBySession(const std::vector<RecordedAction>& actions);
+
+  // Extract candidate subsequences from a session
+  std::vector<std::vector<RecordedAction>> ExtractSubsequences(
+      const std::vector<RecordedAction>& session_actions);
+
+  // Normalize a sequence for pattern matching
+  // (strips variable data like specific input values, timestamps)
+  std::vector<RecordedAction> NormalizeSequence(
+      const std::vector<RecordedAction>& actions);
+
+  // Generate hash for a normalized sequence
+  std::string HashSequence(const std::vector<RecordedAction>& actions);
+
+  // Calculate confidence score for a candidate pattern
+  double CalculateConfidence(const CandidatePattern& candidate);
+
+  // Generate human-readable name for a pattern
+  std::string GeneratePatternName(const std::vector<RecordedAction>& actions);
+
+  // Filter candidates by threshold
+  std::vector<ActionSequence> FilterByThreshold(
+      const std::unordered_map<std::string, CandidatePattern>& candidates);
+
+  // Check if pattern is already dismissed
+  bool IsPatternDismissed(const std::string& pattern_hash);
+
+  // Check if pattern is already converted to workflow
+  bool IsPatternConverted(const std::string& pattern_hash);
+
+  // Notify observers of new pattern
+  void NotifyPatternDetected(const ActionSequence& pattern);
+
+  // Dependencies
+  raw_ptr<ActionStore> action_store_;
+  raw_ptr<PrefService> pref_service_;
+
+  // Configuration
+  int min_sequence_length_ = 3;   // Minimum actions in a pattern
+  int max_sequence_length_ = 20;  // Maximum actions in a pattern
+  int min_occurrences_ = 3;       // Minimum times pattern must occur
+  double min_confidence_ = 0.8;   // Minimum confidence score (0.0 - 1.0)
+
+  // Observers
+  base::ObserverList<PatternDetectorObserver> observers_;
+
+  // Weak pointer factory
+  base::WeakPtrFactory<PatternDetector> weak_factory_{this};
+};
+
+// Utility functions for pattern matching
+
+// Check if two action sequences are similar enough to be the same pattern
+bool AreSequencesSimilar(const std::vector<RecordedAction>& seq1,
+                          const std::vector<RecordedAction>& seq2,
+                          double threshold = 0.9);
+
+// Calculate similarity score between two sequences (0.0 - 1.0)
+double CalculateSequenceSimilarity(const std::vector<RecordedAction>& seq1,
+                                    const std::vector<RecordedAction>& seq2);
+
+}  // namespace browseros::ghost_mode
+
+#endif  // CHROME_BROWSER_BROWSEROS_GHOST_MODE_PATTERN_DETECTOR_H_
