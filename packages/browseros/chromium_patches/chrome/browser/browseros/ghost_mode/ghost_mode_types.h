diff --git a/chrome/browser/browseros/ghost_mode/ghost_mode_types.h b/chrome/browser/browseros/ghost_mode/ghost_mode_types.h
new file mode 100644
index 0000000000000..2a3b4c5d6e7f8
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/ghost_mode_types.h
@@ -0,0 +1,136 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_TYPES_H_
+#define CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_TYPES_H_
+
+#include <string>
+#include <vector>
+
+#include "base/time/time.h"
+#include "base/values.h"
+#include "url/gurl.h"
+
+namespace browseros::ghost_mode {
+
+// Type of user action that can be recorded
+enum class ActionType {
+  kClick,           // Mouse click on an element
+  kType,            // Typing into an input field
+  kNavigate,        // Navigation to a URL
+  kScroll,          // Scrolling the page
+  kSelect,          // Selecting from a dropdown
+  kSubmit,          // Form submission
+  kKeyPress,        // Special key press (Enter, Tab, etc.)
+  kHover,           // Hover over element (for dropdowns)
+  kDragDrop,        // Drag and drop action
+};
+
+// Convert ActionType to string for serialization
+std::string ActionTypeToString(ActionType type);
+
+// Parse ActionType from string
+ActionType StringToActionType(const std::string& str);
+
+// Represents a single recorded user action
+struct RecordedAction {
+  // Unique identifier for this action
+  std::string id;
+  
+  // Type of action performed
+  ActionType type;
+  
+  // URL where action occurred (normalized for pattern matching)
+  GURL url;
+  
+  // URL pattern for matching (domain + path, no query params)
+  std::string url_pattern;
+  
+  // CSS selector(s) for the target element
+  // We store multiple selectors for robustness:
+  // - Primary: data-testid or id based
+  // - Fallback: aria-label or role based
+  // - Last resort: nth-child path
+  std::vector<std::string> selectors;
+  
+  // Text content or aria-label of element (for identification)
+  std::string element_text;
+  
+  // Value for type/select actions
+  // Note: Sensitive values are NEVER stored (see SensitiveDetector)
+  std::string value;
+  
+  // Whether the value is parameterizable (user might want to change it)
+  bool is_parameterizable = false;
+  
+  // Timestamp when action occurred
+  base::Time timestamp;
+  
+  // Tab ID where action occurred
+  int tab_id = -1;
+  
+  // Session ID (groups actions in same browsing session)
+  std::string session_id;
+  
+  // Time since previous action in sequence (for timing patterns)
+  base::TimeDelta time_since_previous;
+  
+  // Additional metadata (viewport size, scroll position, etc.)
+  base::Value::Dict metadata;
+  
+  // Serialize to Value for storage
+  base::Value::Dict ToValue() const;
+  
+  // Deserialize from Value
+  static std::optional<RecordedAction> FromValue(const base::Value::Dict& dict);
+};
+
+// Represents a detected pattern of repeated actions
+struct ActionSequence {
+  // Unique identifier
+  std::string id;
+  
+  // Human-readable name (auto-generated or user-provided)
+  std::string name;
+  
+  // The sequence of actions that form this pattern
+  std::vector<RecordedAction> actions;
+  
+  // Number of times this pattern has been detected
+  int occurrence_count = 0;
+  
+  // When pattern was first seen
+  base::Time first_seen;
+  
+  // When pattern was last seen
+  base::Time last_seen;
+  
+  // Confidence score (0.0 - 1.0)
+  // Based on consistency, selector stability, completion rate
+  double confidence_score = 0.0;
+  
+  // Hash of normalized action sequence (for quick comparison)
+  std::string pattern_hash;
+  
+  // Whether user has dismissed this suggestion
+  bool is_dismissed = false;
+  
+  // Whether user has converted this to a workflow
+  bool is_converted = false;
+  
+  // ID of workflow if converted
+  std::string workflow_id;
+  
+  // Serialize to Value for storage
+  base::Value::Dict ToValue() const;
+  
+  // Deserialize from Value
+  static std::optional<ActionSequence> FromValue(const base::Value::Dict& dict);
+  
+  // Generate a human-readable summary of actions
+  std::vector<std::string> GetActionSummary() const;
+};
+
+}  // namespace browseros::ghost_mode
+
+#endif  // CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_TYPES_H_
