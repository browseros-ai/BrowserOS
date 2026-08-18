diff --git a/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model_browsertest.cc b/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model_browsertest.cc
index 401f627fb1405bfb55cd888a02d4bb4f0d2422de..8875821ed4fea6a63772a89554694148a06233ad 100644
--- a/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model_browsertest.cc
+++ b/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model_browsertest.cc
@@ -7,6 +7,7 @@
 #include <memory>
 
 #include "base/test/scoped_feature_list.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
 #include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/browser.h"
 #include "chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model_factory.h"
@@ -15,6 +16,7 @@
 #include "chrome/common/pref_names.h"
 #include "chrome/test/base/in_process_browser_test.h"
 #include "chrome/test/base/testing_profile.h"
+#include "components/prefs/pref_service.h"
 #include "content/public/test/browser_test.h"
 #include "testing/gtest/include/gtest/gtest.h"
 #include "ui/actions/action_id.h"
@@ -397,5 +399,20 @@ IN_PROC_BROWSER_TEST_F(
   EXPECT_FALSE(model()->Contains(kActionSidePanelShowTabsFromOtherDevices));
 }
 
+IN_PROC_BROWSER_TEST_F(PinnedToolbarActionsModelBrowserTest,
+                       BrowserOSAssistantVisibilityPrefControlsPinnedState) {
+  PrefService* prefs = browser()->profile()->GetPrefs();
+  EXPECT_TRUE(prefs->GetBoolean(browseros::prefs::kShowAssistant));
+
+  model()->EnsureAlwaysPinnedActions();
+  EXPECT_TRUE(model()->Contains(kActionBrowserOSAgent));
+
+  prefs->SetBoolean(browseros::prefs::kShowAssistant, false);
+  EXPECT_FALSE(model()->Contains(kActionBrowserOSAgent));
+
+  prefs->SetBoolean(browseros::prefs::kShowAssistant, true);
+  EXPECT_TRUE(model()->Contains(kActionBrowserOSAgent));
+}
+
 // TODO(dljames): Write tests for guest and incognito mode profile that check
 // that we cannot modify the model at all.
