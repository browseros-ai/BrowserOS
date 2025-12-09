diff --git a/chrome/browser/ui/actions/browseros_actions_config.h b/chrome/browser/ui/actions/browseros_actions_config.h
new file mode 100644
index 0000000000000..bc33293c0e432
--- /dev/null
+++ b/chrome/browser/ui/actions/browseros_actions_config.h
@@ -0,0 +1,73 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_UI_ACTIONS_BROWSEROS_ACTIONS_CONFIG_H_
+#define CHROME_BROWSER_UI_ACTIONS_BROWSEROS_ACTIONS_CONFIG_H_
+
+#include <string>
+#include <string_view>
+
+#include "base/containers/contains.h"
+#include "base/containers/fixed_flat_set.h"
+#include "chrome/browser/ui/actions/chrome_action_id.h"
+#include "chrome/browser/ui/ui_features.h"
+#include "chrome/browser/ui/views/side_panel/side_panel_entry_key.h"
+#include "chrome/common/chrome_features.h"
+#include "chrome/browser/extensions/browseros_extension_constants.h"
+#include "ui/actions/actions.h"
+
+namespace browseros {
+
+// Native action IDs for BrowserOS panels that need special treatment
+// These actions will:
+// - Always be pinned
+// - Show text labels
+// - Have high flex priority (always visible)
+constexpr auto kBrowserOSNativeActionIds =
+    base::MakeFixedFlatSet<actions::ActionId>({
+        kActionSidePanelShowThirdPartyLlm,
+        kActionSidePanelShowClashOfGpts,
+        kActionBrowserOSAgent,
+    });
+
+// Check if an action ID is a BrowserOS action (native or extension)
+inline bool IsBrowserOSAction(actions::ActionId id) {
+  // Check native actions
+  if (kBrowserOSNativeActionIds.contains(id)) {
+    return true;
+  }
+
+  // Only labelled extensions are considered for BrowserOS actions
+  for (const auto& ext_id :
+       extensions::browseros::GetBrowserOSExtensionIds()) {
+    if (!extensions::browseros::IsBrowserOSLabelledExtension(ext_id)) {
+      continue;
+    }
+    auto ext_action_id = actions::ActionIdMap::StringToActionId(
+        SidePanelEntryKey(SidePanelEntryId::kExtension, ext_id)
+            .ToString());
+    if (ext_action_id && id == *ext_action_id) {
+      return true;
+    }
+  }
+
+  return false;
+}
+
+
+// Get the feature flag for a native BrowserOS action
+inline const base::Feature* GetFeatureForBrowserOSAction(actions::ActionId id) {
+  switch (id) {
+    case kActionSidePanelShowThirdPartyLlm:
+      return &features::kThirdPartyLlmPanel;
+    case kActionSidePanelShowClashOfGpts:
+      return &features::kClashOfGpts;
+    default:
+      return nullptr;
+  }
+}
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_UI_ACTIONS_BROWSEROS_ACTIONS_CONFIG_H_
