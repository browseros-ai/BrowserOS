diff --git a/chrome/browser/browseros/ghost_mode/ghost_mode_prefs.cc b/chrome/browser/browseros/ghost_mode/ghost_mode_prefs.cc
new file mode 100644
index 0000000000000..4c5d6e7f8a9b0
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/ghost_mode_prefs.cc
@@ -0,0 +1,105 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_prefs.h"
+
+#include "base/json/json_reader.h"
+#include "base/values.h"
+#include "components/pref_registry/pref_registry_syncable.h"
+#include "components/prefs/pref_service.h"
+
+namespace browseros::ghost_mode {
+
+namespace {
+
+// Default excluded domains (sensitive sites)
+// Users can add more via settings
+const char* kDefaultExcludedDomains[] = {
+    // Banking
+    "*.bank.com",
+    "*.chase.com",
+    "*.wellsfargo.com",
+    "*.bankofamerica.com",
+    // Healthcare
+    "*.healthcare.gov",
+    "*.anthem.com",
+    // Government
+    "*.irs.gov",
+    "*.ssa.gov",
+    // Payment
+    "*.paypal.com",
+    "*.venmo.com",
+};
+
+}  // namespace
+
+void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry) {
+  // Ghost Mode is opt-in by default
+  registry->RegisterBooleanPref(prefs::kGhostModeEnabled, false);
+  
+  // Show onboarding prompt once
+  registry->RegisterBooleanPref(prefs::kGhostModeOnboardingShown, false);
+  
+  // 30 day retention
+  registry->RegisterIntegerPref(prefs::kGhostModeRetentionDays, 30);
+  
+  // Need 3 occurrences to suggest
+  registry->RegisterIntegerPref(prefs::kGhostModeMinOccurrences, 3);
+  
+  // 80% confidence threshold
+  registry->RegisterDoublePref(prefs::kGhostModeMinConfidence, 0.8);
+  
+  // Empty excluded domains (defaults are handled separately)
+  registry->RegisterStringPref(prefs::kGhostModeExcludedDomains, "[]");
+  
+  // Empty dismissed patterns
+  registry->RegisterStringPref(prefs::kGhostModeDismissedPatterns, "[]");
+  
+  // Disabled in incognito by default
+  registry->RegisterBooleanPref(prefs::kGhostModeInIncognito, false);
+}
+
+bool IsGhostModeEnabled(PrefService* pref_service) {
+  return pref_service->GetBoolean(prefs::kGhostModeEnabled);
+}
+
+int GetRetentionDays(PrefService* pref_service) {
+  return pref_service->GetInteger(prefs::kGhostModeRetentionDays);
+}
+
+int GetMinOccurrences(PrefService* pref_service) {
+  return pref_service->GetInteger(prefs::kGhostModeMinOccurrences);
+}
+
+double GetMinConfidence(PrefService* pref_service) {
+  return pref_service->GetDouble(prefs::kGhostModeMinConfidence);
+}
+
+std::vector<std::string> GetExcludedDomains(PrefService* pref_service) {
+  std::vector<std::string> domains;
+  
+  // Add default excluded domains
+  for (const char* domain : kDefaultExcludedDomains) {
+    domains.push_back(domain);
+  }
+  
+  // Add user-configured excluded domains
+  std::string json = pref_service->GetString(prefs::kGhostModeExcludedDomains);
+  auto parsed = base::JSONReader::Read(json);
+  if (parsed && parsed->is_list()) {
+    for (const auto& item : parsed->GetList()) {
+      if (item.is_string()) {
+        domains.push_back(item.GetString());
+      }
+    }
+  }
+  
+  return domains;
+}
+
+bool IsDomainExcluded(PrefService* pref_service, const std::string& domain) {
+  // TODO: Implement wildcard matching
+  auto excluded = GetExcludedDomains(pref_service);
+  return std::find(excluded.begin(), excluded.end(), domain) != excluded.end();
+}
+
+}  // namespace browseros::ghost_mode
