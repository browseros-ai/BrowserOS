diff --git a/chrome/browser/browseros/ghost_mode/pattern_detector.cc b/chrome/browser/browseros/ghost_mode/pattern_detector.cc
new file mode 100644
index 0000000000000..b2c3d4e5f6a7b
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/pattern_detector.cc
@@ -0,0 +1,382 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/pattern_detector.h"
+
+#include <algorithm>
+#include <functional>
+
+#include "base/hash/hash.h"
+#include "base/logging.h"
+#include "base/strings/string_util.h"
+#include "base/task/thread_pool.h"
+#include "base/uuid.h"
+#include "chrome/browser/browseros/ghost_mode/action_store.h"
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_prefs.h"
+#include "components/prefs/pref_service.h"
+
+namespace browseros::ghost_mode {
+
+namespace {
+
+// Session gap threshold - actions more than 30 minutes apart are in different sessions
+constexpr base::TimeDelta kSessionGapThreshold = base::Minutes(30);
+
+// Maximum time span to analyze (look back 7 days by default)
+constexpr base::TimeDelta kDefaultAnalysisWindow = base::Days(7);
+
+}  // namespace
+
+PatternDetector::PatternDetector(ActionStore* action_store,
+                                  PrefService* pref_service)
+    : action_store_(action_store), pref_service_(pref_service) {
+  CHECK(action_store_);
+  CHECK(pref_service_);
+  
+  // Load configuration from prefs
+  min_occurrences_ = pref_service_->GetInteger(prefs::kGhostModeMinOccurrences);
+  min_confidence_ = pref_service_->GetDouble(prefs::kGhostModeMinConfidence);
+}
+
+PatternDetector::~PatternDetector() = default;
+
+std::vector<ActionSequence> PatternDetector::DetectPatterns() {
+  VLOG(1) << "browseros: Starting pattern detection";
+  
+  // Get actions from the analysis window
+  base::Time end = base::Time::Now();
+  base::Time start = end - kDefaultAnalysisWindow;
+  
+  std::vector<RecordedAction> all_actions =
+      action_store_->GetActionsInRange(start, end);
+  
+  if (all_actions.empty()) {
+    VLOG(1) << "browseros: No actions to analyze";
+    return {};
+  }
+  
+  VLOG(1) << "browseros: Analyzing " << all_actions.size() << " actions";
+  
+  // Group actions by session
+  auto sessions = GroupBySession(all_actions);
+  VLOG(1) << "browseros: Found " << sessions.size() << " sessions";
+  
+  // Collect all candidate patterns
+  std::unordered_map<std::string, CandidatePattern> candidates;
+  
+  for (const auto& [session_id, session_actions] : sessions) {
+    // Skip very short sessions
+    if (session_actions.size() < static_cast<size_t>(min_sequence_length_)) {
+      continue;
+    }
+    
+    // Extract subsequences from this session
+    auto subsequences = ExtractSubsequences(session_actions);
+    
+    for (const auto& subseq : subsequences) {
+      // Normalize the sequence for pattern matching
+      auto normalized = NormalizeSequence(subseq);
+      
+      // Generate hash for this normalized sequence
+      std::string hash = HashSequence(normalized);
+      
+      // Add to candidates or update existing
+      auto it = candidates.find(hash);
+      if (it != candidates.end()) {
+        it->second.occurrence_times.push_back(subseq.front().timestamp);
+        // Keep the first occurrence's actions as representative
+      } else {
+        CandidatePattern candidate;
+        candidate.hash = hash;
+        candidate.actions = normalized;
+        candidate.occurrence_times.push_back(subseq.front().timestamp);
+        candidates[hash] = std::move(candidate);
+      }
+    }
+  }
+  
+  VLOG(1) << "browseros: Found " << candidates.size() << " unique sequences";
+  
+  // Calculate confidence scores and filter by threshold
+  for (auto& [hash, candidate] : candidates) {
+    candidate.confidence_score = CalculateConfidence(candidate);
+  }
+  
+  auto patterns = FilterByThreshold(candidates);
+  
+  VLOG(1) << "browseros: " << patterns.size() << " patterns meet threshold";
+  
+  // Notify observers
+  for (auto& observer : observers_) {
+    observer.OnDetectionComplete(static_cast<int>(patterns.size()));
+  }
+  
+  for (const auto& pattern : patterns) {
+    NotifyPatternDetected(pattern);
+  }
+  
+  return patterns;
+}
+
+void PatternDetector::DetectPatternsAsync(
+    base::OnceCallback<void(std::vector<ActionSequence>)> callback) {
+  base::ThreadPool::PostTaskAndReplyWithResult(
+      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_VISIBLE},
+      base::BindOnce(&PatternDetector::DetectPatterns,
+                     weak_factory_.GetWeakPtr()),
+      std::move(callback));
+}
+
+bool PatternDetector::HasExistingPattern(
+    const std::vector<RecordedAction>& actions) {
+  auto normalized = NormalizeSequence(actions);
+  std::string hash = HashSequence(normalized);
+  
+  // Check if this pattern is already saved
+  auto existing = action_store_->GetAllPatterns();
+  for (const auto& pattern : existing) {
+    if (pattern.pattern_hash == hash) {
+      return true;
+    }
+  }
+  
+  return false;
+}
+
+void PatternDetector::AddObserver(PatternDetectorObserver* observer) {
+  observers_.AddObserver(observer);
+}
+
+void PatternDetector::RemoveObserver(PatternDetectorObserver* observer) {
+  observers_.RemoveObserver(observer);
+}
+
+std::unordered_map<std::string, std::vector<RecordedAction>>
+PatternDetector::GroupBySession(const std::vector<RecordedAction>& actions) {
+  std::unordered_map<std::string, std::vector<RecordedAction>> sessions;
+  
+  if (actions.empty()) {
+    return sessions;
+  }
+  
+  // First, group by explicit session_id
+  for (const auto& action : actions) {
+    sessions[action.session_id].push_back(action);
+  }
+  
+  // Then, split sessions by time gaps
+  std::unordered_map<std::string, std::vector<RecordedAction>> result;
+  int subsession_counter = 0;
+  
+  for (auto& [session_id, session_actions] : sessions) {
+    // Sort by timestamp
+    std::sort(session_actions.begin(), session_actions.end(),
+              [](const RecordedAction& a, const RecordedAction& b) {
+                return a.timestamp < b.timestamp;
+              });
+    
+    // Split by time gaps - use UUID for unique subsession IDs
+    std::string current_session = session_id + "_" +
+                                   base::Uuid::GenerateRandomV4().AsLowercaseString();
+    result[current_session].push_back(session_actions[0]);
+    
+    for (size_t i = 1; i < session_actions.size(); ++i) {
+      base::TimeDelta gap =
+          session_actions[i].timestamp - session_actions[i - 1].timestamp;
+      
+      if (gap > kSessionGapThreshold) {
+        // Start a new subsession with UUID for uniqueness
+        current_session = session_id + "_" +
+                           base::Uuid::GenerateRandomV4().AsLowercaseString();
+      }
+      
+      result[current_session].push_back(session_actions[i]);
+    }
+  }
+  
+  return result;
+}
+
+std::vector<std::vector<RecordedAction>> PatternDetector::ExtractSubsequences(
+    const std::vector<RecordedAction>& session_actions) {
+  std::vector<std::vector<RecordedAction>> subsequences;
+  
+  size_t n = session_actions.size();
+  
+  // Extract all subsequences of valid lengths
+  for (int len = min_sequence_length_; len <= max_sequence_length_; ++len) {
+    if (static_cast<size_t>(len) > n) {
+      break;
+    }
+    
+    for (size_t start = 0; start <= n - len; ++start) {
+      std::vector<RecordedAction> subseq;
+      subseq.reserve(len);
+      
+      for (int i = 0; i < len; ++i) {
+        subseq.push_back(session_actions[start + i]);
+      }
+      
+      subsequences.push_back(std::move(subseq));
+    }
+  }
+  
+  return subsequences;
+}
+
+std::vector<RecordedAction> PatternDetector::NormalizeSequence(
+    const std::vector<RecordedAction>& actions) {
+  std::vector<RecordedAction> normalized;
+  normalized.reserve(actions.size());
+  
+  for (const auto& action : actions) {
+    RecordedAction norm_action;
+    norm_action.type = action.type;
+    
+    // Normalize URL - keep domain and path, remove query params
+    if (action.url.is_valid()) {
+      GURL::Replacements replacements;
+      replacements.ClearQuery();
+      replacements.ClearRef();
+      norm_action.url = action.url.ReplaceComponents(replacements);
+    }
+    norm_action.url_pattern = action.url_pattern;
+    
+    // Keep selectors (already should be stable)
+    norm_action.selectors = action.selectors;
+    
+    // Keep element text for context
+    norm_action.element_text = action.element_text;
+    
+    // For type actions, only keep that it was a type action
+    // (actual values might vary)
+    if (action.type == ActionType::kType) {
+      norm_action.value = "[input]";  // Placeholder
+      norm_action.is_parameterizable = true;
+    } else {
+      norm_action.value = action.value;
+      norm_action.is_parameterizable = action.is_parameterizable;
+    }
+    
+    // Don't include timestamp in normalization
+    // Don't include tab_id or session_id
+    
+    normalized.push_back(std::move(norm_action));
+  }
+  
+  return normalized;
+}
+
+std::string PatternDetector::HashSequence(
+    const std::vector<RecordedAction>& actions) {
+  std::string combined;
+  
+  for (const auto& action : actions) {
+    combined += ActionTypeToString(action.type);
+    combined += "|";
+    combined += action.url_pattern;
+    combined += "|";
+    
+    // Use first selector for hashing
+    if (!action.selectors.empty()) {
+      combined += action.selectors[0];
+    }
+    combined += "|";
+    
+    // Don't include value in hash (parameterizable)
+    combined += action.element_text;
+    combined += "||";  // Separator between actions
+  }
+  
+  // Generate hash
+  size_t hash = base::FastHash(base::as_byte_span(combined));
+  return base::NumberToString(hash);
+}
+
+double PatternDetector::CalculateConfidence(const CandidatePattern& candidate) {
+  double confidence = 0.0;
+  
+  // Factor 1: Occurrence count (more is better, up to a point)
+  size_t occurrences = candidate.occurrence_times.size();
+  double occurrence_score = std::min(1.0, occurrences / 10.0);
+  
+  // Factor 2: Time distribution (regular intervals are better)
+  double distribution_score = 0.5;  // Default for 1-2 occurrences
+  if (occurrences >= 3) {
+    // Calculate variance in time gaps
+    std::vector<double> gaps;
+    auto times = candidate.occurrence_times;
+    std::sort(times.begin(), times.end());
+    
+    for (size_t i = 1; i < times.size(); ++i) {
+      gaps.push_back((times[i] - times[i - 1]).InMinutes());
+    }
+    
+    if (!gaps.empty()) {
+      double mean = 0.0;
+      for (double gap : gaps) {
+        mean += gap;
+      }
+      mean /= gaps.size();
+      
+      double variance = 0.0;
+      for (double gap : gaps) {
+        variance += (gap - mean) * (gap - mean);
+      }
+      variance /= gaps.size();
+      
+      // Lower variance = more regular = higher score
+      double std_dev = std::sqrt(variance);
+      double cv = (mean > 0) ? std_dev / mean : 1.0;
+      distribution_score = std::max(0.0, 1.0 - cv);
+    }
+  }
+  
+  // Factor 3: Selector stability (data-testid > id > class > nth-child)
+  double selector_score = 0.0;
+  int stable_selectors = 0;
+  for (const auto& action : candidate.actions) {
+    if (!action.selectors.empty()) {
+      const std::string& sel = action.selectors[0];
+      if (sel.find("data-testid") != std::string::npos ||
+          sel.find("data-test") != std::string::npos) {
+        stable_selectors += 3;
+      } else if (sel.find("#") != std::string::npos) {
+        stable_selectors += 2;
+      } else if (sel.find("[aria-") != std::string::npos) {
+        stable_selectors += 2;
+      } else {
+        stable_selectors += 1;
+      }
+    }
+  }
+  selector_score = std::min(1.0, stable_selectors /
+                                      (3.0 * candidate.actions.size()));
+  
+  // Combine factors with weights
+  confidence = (occurrence_score * 0.4) +
+               (distribution_score * 0.2) +
+               (selector_score * 0.4);
+  
+  return confidence;
+}
+
+std::string PatternDetector::GeneratePatternName(
+    const std::vector<RecordedAction>& actions) {
+  if (actions.empty()) {
+    return "Unknown Pattern";
+  }
+  
+  // Use the first action's URL domain and last action type
+  std::string domain;
+  if (actions[0].url.is_valid()) {
+    domain = actions[0].url.host();
+  } else {
+    domain = "web";
+  }
+  
+  std::string last_action = ActionTypeToString(actions.back().type);
+  
+  return domain + " - " + last_action + " flow (" +
+         base::NumberToString(actions.size()) + " steps)";
+}
+
+std::vector<ActionSequence> PatternDetector::FilterByThreshold(
+    const std::unordered_map<std::string, CandidatePattern>& candidates) {
+  std::vector<ActionSequence> patterns;
+  
+  for (const auto& [hash, candidate] : candidates) {
+    // Check occurrence count
+    if (static_cast<int>(candidate.occurrence_times.size()) < min_occurrences_) {
+      continue;
+    }
+    
+    // Check confidence score
+    if (candidate.confidence_score < min_confidence_) {
+      continue;
+    }
+    
+    // Check if pattern is already dismissed
+    if (IsPatternDismissed(hash)) {
+      continue;
+    }
+    
+    // Check if pattern is already converted
+    if (IsPatternConverted(hash)) {
+      continue;
+    }
+    
+    // Create ActionSequence from candidate
+    ActionSequence pattern;
+    pattern.id = base::Uuid::GenerateRandomV4().AsLowercaseString();
+    pattern.name = GeneratePatternName(candidate.actions);
+    pattern.actions = candidate.actions;
+    pattern.occurrence_count =
+        static_cast<int>(candidate.occurrence_times.size());
+    
+    auto times = candidate.occurrence_times;
+    std::sort(times.begin(), times.end());
+    pattern.first_seen = times.front();
+    pattern.last_seen = times.back();
+    
+    pattern.confidence_score = candidate.confidence_score;
+    pattern.pattern_hash = hash;
+    pattern.status = PatternStatus::kNew;
+    
+    // Set URL pattern from first action
+    if (!candidate.actions.empty()) {
+      pattern.url_pattern = candidate.actions[0].url_pattern;
+    }
+    
+    patterns.push_back(std::move(pattern));
+  }
+  
+  // Sort by confidence score descending
+  std::sort(patterns.begin(), patterns.end(),
+            [](const ActionSequence& a, const ActionSequence& b) {
+              return a.confidence_score > b.confidence_score;
+            });
+  
+  return patterns;
+}
+
+bool PatternDetector::IsPatternDismissed(const std::string& pattern_hash) {
+  auto existing = action_store_->GetAllPatterns();
+  for (const auto& pattern : existing) {
+    if (pattern.pattern_hash == pattern_hash &&
+        pattern.status == PatternStatus::kDismissed) {
+      return true;
+    }
+  }
+  return false;
+}
+
+bool PatternDetector::IsPatternConverted(const std::string& pattern_hash) {
+  auto existing = action_store_->GetAllPatterns();
+  for (const auto& pattern : existing) {
+    if (pattern.pattern_hash == pattern_hash &&
+        pattern.status == PatternStatus::kConverted) {
+      return true;
+    }
+  }
+  return false;
+}
+
+void PatternDetector::NotifyPatternDetected(const ActionSequence& pattern) {
+  for (auto& observer : observers_) {
+    observer.OnPatternDetected(pattern);
+  }
+}
+
+// Utility functions
+
+bool AreSequencesSimilar(const std::vector<RecordedAction>& seq1,
+                          const std::vector<RecordedAction>& seq2,
+                          double threshold) {
+  return CalculateSequenceSimilarity(seq1, seq2) >= threshold;
+}
+
+double CalculateSequenceSimilarity(const std::vector<RecordedAction>& seq1,
+                                    const std::vector<RecordedAction>& seq2) {
+  if (seq1.size() != seq2.size()) {
+    // Different lengths - calculate based on overlap
+    size_t min_len = std::min(seq1.size(), seq2.size());
+    size_t max_len = std::max(seq1.size(), seq2.size());
+    
+    if (min_len == 0) {
+      return 0.0;
+    }
+    
+    double length_penalty = static_cast<double>(min_len) / max_len;
+    
+    // Compare up to min_len
+    int matches = 0;
+    for (size_t i = 0; i < min_len; ++i) {
+      if (seq1[i].type == seq2[i].type &&
+          seq1[i].url_pattern == seq2[i].url_pattern) {
+        ++matches;
+      }
+    }
+    
+    return (static_cast<double>(matches) / min_len) * length_penalty;
+  }
+  
+  // Same length - direct comparison
+  int matches = 0;
+  for (size_t i = 0; i < seq1.size(); ++i) {
+    // Compare type
+    if (seq1[i].type != seq2[i].type) {
+      continue;
+    }
+    
+    // Compare URL pattern
+    if (seq1[i].url_pattern != seq2[i].url_pattern) {
+      continue;
+    }
+    
+    // Compare selectors (at least one must match)
+    bool selector_match = false;
+    for (const auto& sel1 : seq1[i].selectors) {
+      for (const auto& sel2 : seq2[i].selectors) {
+        if (sel1 == sel2) {
+          selector_match = true;
+          break;
+        }
+      }
+      if (selector_match) {
+        break;
+      }
+    }
+    
+    if (selector_match || seq1[i].selectors.empty()) {
+      ++matches;
+    }
+  }
+  
+  return static_cast<double>(matches) / seq1.size();
+}
+
+}  // namespace browseros::ghost_mode
