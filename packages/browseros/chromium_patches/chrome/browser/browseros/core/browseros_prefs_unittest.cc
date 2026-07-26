diff --git a/chrome/browser/browseros/core/browseros_prefs_unittest.cc b/chrome/browser/browseros/core/browseros_prefs_unittest.cc
new file mode 100644
index 0000000000000..88f9feadd5d8d
--- /dev/null
+++ b/chrome/browser/browseros/core/browseros_prefs_unittest.cc
@@ -0,0 +1,131 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/core/browseros_prefs.h"
+
+#include "base/test/scoped_command_line.h"
+#include "chrome/browser/browseros/buildflags.h"
+#include "chrome/browser/browseros/core/browseros_product.h"
+#include "chrome/browser/browseros/core/browseros_switches.h"
+#include "components/bookmarks/browser/bookmark_utils.h"
+#include "components/bookmarks/common/bookmark_pref_names.h"
+#include "components/pref_registry/pref_registry_syncable.h"
+#include "components/sync_preferences/testing_pref_service_syncable.h"
+#include "testing/gtest/include/gtest/gtest.h"
+
+namespace browseros {
+namespace {
+
+void RegisterPrefs(sync_preferences::TestingPrefServiceSyncable* pref_service) {
+  bookmarks::RegisterProfilePrefs(pref_service->registry());
+  RegisterProfilePrefs(pref_service->registry());
+}
+
+#if BUILDFLAG(BROWSEROS_ALLOW_RUNTIME_PRODUCT_OVERRIDE)
+void SetProductOverride(base::test::ScopedCommandLine* scoped_command_line,
+                        Product product) {
+  scoped_command_line->GetProcessCommandLine()->AppendSwitchASCII(
+      kBrowserOSProduct, product == Product::kBrowserClaw
+                             ? kBrowserClawProductValue
+                             : kBrowserOSProductValue);
+}
+#endif  // BUILDFLAG(BROWSEROS_ALLOW_RUNTIME_PRODUCT_OVERRIDE)
+
+TEST(BrowserOSPrefsTest, ShowTabGroupsInBookmarkBarDefaultMatchesBakedProduct) {
+  sync_preferences::TestingPrefServiceSyncable pref_service;
+  RegisterPrefs(&pref_service);
+
+  EXPECT_EQ(!IsBrowserClawProduct(),
+            pref_service.GetBoolean(prefs::kShowTabGroupsInBookmarkBar));
+}
+
+#if BUILDFLAG(BROWSEROS_ALLOW_RUNTIME_PRODUCT_OVERRIDE)
+TEST(BrowserOSPrefsTest, BrowserClawDefaultsToHidingTabGroupsInBookmarkBar) {
+  base::test::ScopedCommandLine scoped_command_line;
+  SetProductOverride(&scoped_command_line, Product::kBrowserClaw);
+  sync_preferences::TestingPrefServiceSyncable pref_service;
+  RegisterPrefs(&pref_service);
+
+  EXPECT_FALSE(pref_service.GetBoolean(prefs::kShowTabGroupsInBookmarkBar));
+}
+
+TEST(BrowserOSPrefsTest, BrowserOSDefaultsToShowingTabGroupsInBookmarkBar) {
+  base::test::ScopedCommandLine scoped_command_line;
+  SetProductOverride(&scoped_command_line, Product::kBrowserOS);
+  sync_preferences::TestingPrefServiceSyncable pref_service;
+  RegisterPrefs(&pref_service);
+
+  EXPECT_TRUE(pref_service.GetBoolean(prefs::kShowTabGroupsInBookmarkBar));
+}
+#endif  // BUILDFLAG(BROWSEROS_ALLOW_RUNTIME_PRODUCT_OVERRIDE)
+
+TEST(BrowserOSPrefsTest,
+     SyncShowTabGroupsInBookmarkBarPrefAppliesBrowserOSDefault) {
+  sync_preferences::TestingPrefServiceSyncable pref_service;
+  RegisterPrefs(&pref_service);
+  pref_service.SetBoolean(prefs::kShowTabGroupsInBookmarkBar, false);
+
+  ASSERT_TRUE(pref_service
+                  .FindPreference(bookmarks::prefs::kShowTabGroupsInBookmarkBar)
+                  ->IsDefaultValue());
+  SyncShowTabGroupsInBookmarkBarPref(&pref_service);
+
+  EXPECT_FALSE(
+      pref_service.GetBoolean(bookmarks::prefs::kShowTabGroupsInBookmarkBar));
+}
+
+TEST(BrowserOSPrefsTest,
+     SyncShowTabGroupsInBookmarkBarPrefPreservesUserOverride) {
+  sync_preferences::TestingPrefServiceSyncable pref_service;
+  RegisterPrefs(&pref_service);
+  pref_service.SetBoolean(prefs::kShowTabGroupsInBookmarkBar, true);
+  pref_service.SetBoolean(bookmarks::prefs::kShowTabGroupsInBookmarkBar, false);
+
+  ASSERT_FALSE(
+      pref_service
+          .FindPreference(bookmarks::prefs::kShowTabGroupsInBookmarkBar)
+          ->IsDefaultValue());
+  SyncShowTabGroupsInBookmarkBarPref(&pref_service);
+
+  EXPECT_FALSE(
+      pref_service.GetBoolean(bookmarks::prefs::kShowTabGroupsInBookmarkBar));
+}
+
+TEST(BrowserOSPrefsTest,
+     SyncShowTabGroupsInBookmarkBarPrefLeavesMatchingDefaultUntouched) {
+  sync_preferences::TestingPrefServiceSyncable pref_service;
+  RegisterPrefs(&pref_service);
+  pref_service.SetBoolean(prefs::kShowTabGroupsInBookmarkBar, true);
+
+  const PrefService::Preference* upstream_pref = pref_service.FindPreference(
+      bookmarks::prefs::kShowTabGroupsInBookmarkBar);
+  ASSERT_TRUE(upstream_pref->IsDefaultValue());
+  ASSERT_TRUE(
+      pref_service.GetBoolean(bookmarks::prefs::kShowTabGroupsInBookmarkBar));
+
+  SyncShowTabGroupsInBookmarkBarPref(&pref_service);
+
+  EXPECT_TRUE(upstream_pref->IsDefaultValue());
+  EXPECT_TRUE(
+      pref_service.GetBoolean(bookmarks::prefs::kShowTabGroupsInBookmarkBar));
+}
+
+TEST(BrowserOSPrefsTest,
+     ApplyShowTabGroupsInBookmarkBarPrefUpdatesUpstreamPref) {
+  sync_preferences::TestingPrefServiceSyncable pref_service;
+  RegisterPrefs(&pref_service);
+
+  pref_service.SetBoolean(prefs::kShowTabGroupsInBookmarkBar, false);
+  ApplyShowTabGroupsInBookmarkBarPref(&pref_service);
+  EXPECT_FALSE(
+      pref_service.GetBoolean(bookmarks::prefs::kShowTabGroupsInBookmarkBar));
+
+  pref_service.SetBoolean(prefs::kShowTabGroupsInBookmarkBar, true);
+  ApplyShowTabGroupsInBookmarkBarPref(&pref_service);
+  EXPECT_TRUE(
+      pref_service.GetBoolean(bookmarks::prefs::kShowTabGroupsInBookmarkBar));
+}
+
+}  // namespace
+}  // namespace browseros
