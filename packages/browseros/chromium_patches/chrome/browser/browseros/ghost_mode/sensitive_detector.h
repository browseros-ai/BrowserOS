diff --git a/chrome/browser/browseros/ghost_mode/sensitive_detector.h b/chrome/browser/browseros/ghost_mode/sensitive_detector.h
new file mode 100644
index 0000000000000..5d6e7f8a9b0c1
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/sensitive_detector.h
@@ -0,0 +1,78 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_GHOST_MODE_SENSITIVE_DETECTOR_H_
+#define CHROME_BROWSER_BROWSEROS_GHOST_MODE_SENSITIVE_DETECTOR_H_
+
+#include <string>
+#include <vector>
+
+namespace browseros::ghost_mode {
+
+// SensitiveDetector determines whether a form field or action
+// should NOT be recorded by Ghost Mode for privacy reasons.
+//
+// This is a critical privacy component. When in doubt, we err on
+// the side of NOT recording.
+class SensitiveDetector {
+ public:
+  SensitiveDetector();
+  ~SensitiveDetector();
+
+  // Check if an input field is sensitive based on its attributes
+  // Returns true if the field should NOT be recorded
+  bool IsSensitiveField(const std::string& input_type,
+                        const std::string& name,
+                        const std::string& id,
+                        const std::string& autocomplete,
+                        const std::string& aria_label,
+                        const std::string& placeholder) const;
+
+  // Check if a CSS selector path indicates a sensitive field
+  bool IsSensitiveSelector(const std::string& selector) const;
+
+  // Check if a URL is for a sensitive page (login, payment, etc.)
+  bool IsSensitiveUrl(const std::string& url) const;
+
+  // Check if element text/label suggests sensitivity
+  bool IsSensitiveLabel(const std::string& label) const;
+
+ private:
+  // Input types that are always sensitive
+  static const std::vector<std::string> kSensitiveInputTypes;
+
+  // Name/ID patterns that indicate sensitivity
+  static const std::vector<std::string> kSensitiveNamePatterns;
+
+  // Autocomplete values that indicate sensitivity
+  static const std::vector<std::string> kSensitiveAutocompleteValues;
+
+  // CSS class/ID patterns that indicate sensitive forms
+  static const std::vector<std::string> kSensitiveSelectorPatterns;
+
+  // URL path patterns that indicate sensitive pages
+  static const std::vector<std::string> kSensitiveUrlPatterns;
+
+  // Label text patterns that indicate sensitivity
+  static const std::vector<std::string> kSensitiveLabelPatterns;
+
+  // Helper to check if string contains any pattern (case-insensitive)
+  bool ContainsAnyPattern(const std::string& str,
+                          const std::vector<std::string>& patterns) const;
+};
+
+// Singleton accessor for the detector
+SensitiveDetector& GetSensitiveDetector();
+
+// Convenience function to check if recording should be skipped
+// This is the main entry point used by ActionRecorder
+bool ShouldSkipRecording(const std::string& input_type,
+                         const std::string& name,
+                         const std::string& id,
+                         const std::string& autocomplete,
+                         const std::string& aria_label,
+                         const std::string& placeholder,
+                         const std::string& selector,
+                         const std::string& url);
+
+}  // namespace browseros::ghost_mode
+
+#endif  // CHROME_BROWSER_BROWSEROS_GHOST_MODE_SENSITIVE_DETECTOR_H_
