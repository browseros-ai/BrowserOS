diff --git a/chrome/browser/browseros/customization/command_line_prefs.cc b/chrome/browser/browseros/customization/command_line_prefs.cc
new file mode 100644
index 0000000000000000000000000000000000000000..00ef84c895014002267544df1e3c63ed9c2ed15b
--- /dev/null
+++ b/chrome/browser/browseros/customization/command_line_prefs.cc
@@ -0,0 +1,91 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/customization/command_line_prefs.h"
+
+#include <string>
+#include <string_view>
+#include <vector>
+
+#include "base/command_line.h"
+#include "base/feature_list.h"
+#include "base/logging.h"
+#include "base/strings/string_util.h"
+#include "base/values.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
+#include "chrome/browser/browseros/customization/customization_pref_names.h"
+#include "chrome/common/pref_names.h"
+#include "components/bookmarks/common/bookmark_pref_names.h"
+#include "components/prefs/writeable_pref_store.h"
+
+namespace browseros {
+
+namespace {
+
+// One pref that a --disable-browseros-features name forces while it is listed.
+// Like Chromium's own switch-to-pref tables, a row only points at a pref; the
+// pref and the UI that reads it live elsewhere.
+struct ForcedPref {
+  std::string_view feature;
+  std::string_view pref;
+  bool value;
+};
+
+// Rows name prefs through their exported constants, never string literals, so
+// a Chromium rebase that renames or removes a pref breaks the build instead of
+// leaving a row that silently does nothing.
+constexpr ForcedPref kDisabledFeaturePrefs[] = {
+    {"VerticalTabs", ::prefs::kVerticalTabsEnabled, false},
+    // Chromium offers a "try vertical tabs" promo until this is set. Its
+    // tutorial would lead to the toggle the forced layout pref has locked.
+    {"VerticalTabs", ::prefs::kVerticalTabsEnabledFirstTime, true},
+    // BrowserOS's customization screen shows this pref, not Chromium's.
+    {"VerticalTabs", browseros::prefs::kVerticalTabsEnabled, false},
+    {"HomeButton", ::prefs::kShowHomeButton, false},
+    {"SplitTabButton", ::prefs::kPinSplitTabButton, false},
+    {"TabSearchButton", ::prefs::kTabSearchPinnedToTabstrip, false},
+    {"TabSearchButton", ::prefs::kEverythingMenuPinnedToTabstrip, false},
+    // Makes all bookmarks read-only, which also removes the star.
+    {"BookmarkEditing", bookmarks::prefs::kEditBookmarksEnabled, false},
+    {"WebAppInstall", ::prefs::kWebAppInstallByUserEnabled, false},
+    {"AvatarButton", browseros::prefs::kShowAvatarButton, false},
+    {"ExtensionsToolbar", browseros::prefs::kShowExtensionsToolbar, false},
+};
+
+std::string KnownDisabledFeatureNames() {
+  std::vector<std::string_view> names;
+  for (const ForcedPref& row : kDisabledFeaturePrefs) {
+    if (names.empty() || names.back() != row.feature) {
+      names.push_back(row.feature);
+    }
+  }
+  return base::JoinString(names, ",");
+}
+
+}  // namespace
+
+void ApplyCommandLinePrefs(const base::CommandLine& command_line,
+                           WriteablePrefStore& store) {
+  // UTF-8 rather than ASCII: the ASCII getter drops the whole value when any
+  // character isn't ASCII, which would discard valid names along with a typo.
+  const std::string disabled_features =
+      command_line.GetSwitchValueUTF8(switches::kDisableBrowserOSFeatures);
+  for (std::string_view name :
+       base::FeatureList::SplitFeatureListString(disabled_features)) {
+    bool known = false;
+    for (const ForcedPref& row : kDisabledFeaturePrefs) {
+      if (row.feature == name) {
+        store.SetValue(row.pref, base::Value(row.value),
+                       WriteablePrefStore::DEFAULT_PREF_WRITE_FLAGS);
+        known = true;
+      }
+    }
+    LOG_IF(WARNING, !known)
+        << "Ignoring unknown --" << switches::kDisableBrowserOSFeatures
+        << " name \"" << name
+        << "\". Known names: " << KnownDisabledFeatureNames();
+  }
+}
+
+}  // namespace browseros
