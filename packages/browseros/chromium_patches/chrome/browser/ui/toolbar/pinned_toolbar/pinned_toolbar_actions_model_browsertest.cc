diff --git a/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model_browsertest.cc b/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model_browsertest.cc
index e75fd9b4972b3..8a1cd615d2897 100644
--- a/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model_browsertest.cc
+++ b/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model_browsertest.cc
@@ -6,6 +6,7 @@
 
 #include <memory>
 
+#include "chrome/browser/browseros/core/browseros_prefs.h"
 #include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/browser.h"
 #include "chrome/browser/ui/tab_search_feature.h"
@@ -14,6 +15,7 @@
 #include "chrome/common/pref_names.h"
 #include "chrome/test/base/in_process_browser_test.h"
 #include "chrome/test/base/testing_profile.h"
+#include "components/prefs/pref_service.h"
 #include "content/public/test/browser_test.h"
 #include "testing/gtest/include/gtest/gtest.h"
 #include "ui/actions/action_id.h"
@@ -451,5 +453,20 @@ IN_PROC_BROWSER_TEST_F(PinnedToolbarActionsModelBrowserTest,
   }
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
