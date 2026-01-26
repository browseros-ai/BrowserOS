diff --git a/chrome/browser/browseros/ghost_mode/ghost_mode_prefs.h b/chrome/browser/browseros/ghost_mode/ghost_mode_prefs.h
new file mode 100644
index 0000000000000..3b4c5d6e7f8a9
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/ghost_mode_prefs.h
@@ -0,0 +1,73 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_PREFS_H_
+#define CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_PREFS_H_
+
+#include <string>
+#include <vector>
+
+class PrefRegistrySimple;
+class PrefService;
+
+namespace user_prefs {
+class PrefRegistrySyncable;
+}
+
+namespace browseros::ghost_mode {
+
+namespace prefs {
+
+// Whether Ghost Mode is enabled (bool)
+// Default: false (opt-in)
+inline constexpr char kGhostModeEnabled[] = "browseros.ghost_mode.enabled";
+
+// Whether to show onboarding prompt for Ghost Mode (bool)
+// Default: true (show once)
+inline constexpr char kGhostModeOnboardingShown[] = 
+    "browseros.ghost_mode.onboarding_shown";
+
+// Data retention period in days (int)
+// Default: 30
+inline constexpr char kGhostModeRetentionDays[] = 
+    "browseros.ghost_mode.retention_days";
+
+// Minimum occurrences before suggesting automation (int)
+// Default: 3
+inline constexpr char kGhostModeMinOccurrences[] = 
+    "browseros.ghost_mode.min_occurrences";
+
+// Minimum confidence score to suggest (double, 0.0-1.0)
+// Default: 0.8
+inline constexpr char kGhostModeMinConfidence[] = 
+    "browseros.ghost_mode.min_confidence";
+
+// List of excluded domains (JSON array of strings)
+// Default: [] (empty, some sites auto-excluded like banks)
+inline constexpr char kGhostModeExcludedDomains[] = 
+    "browseros.ghost_mode.excluded_domains";
+
+// List of dismissed pattern IDs (JSON array of strings)
+inline constexpr char kGhostModeDismissedPatterns[] = 
+    "browseros.ghost_mode.dismissed_patterns";
+
+// Enable Ghost Mode in incognito (bool)
+// Default: false (privacy expectation)
+inline constexpr char kGhostModeInIncognito[] = 
+    "browseros.ghost_mode.in_incognito";
+
+}  // namespace prefs
+
+// Register Ghost Mode preferences
+void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry);
+
+// Helper functions
+bool IsGhostModeEnabled(PrefService* pref_service);
+int GetRetentionDays(PrefService* pref_service);
+int GetMinOccurrences(PrefService* pref_service);
+double GetMinConfidence(PrefService* pref_service);
+std::vector<std::string> GetExcludedDomains(PrefService* pref_service);
+bool IsDomainExcluded(PrefService* pref_service, const std::string& domain);
+
+}  // namespace browseros::ghost_mode
+
+#endif  // CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_PREFS_H_
