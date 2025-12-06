diff --git a/chrome/browser/ui/browser_actions.cc b/chrome/browser/ui/browser_actions.cc
index fb3dba200be8c..080fadd58fd7e 100644
--- a/chrome/browser/ui/browser_actions.cc
+++ b/chrome/browser/ui/browser_actions.cc
@@ -20,6 +20,11 @@
 #include "chrome/browser/sharing_hub/sharing_hub_features.h"
 #include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/actions/chrome_actions.h"
+#include "chrome/browser/extensions/api/side_panel/side_panel_service.h"
+#include "chrome/browser/extensions/browseros_extension_constants.h"
+#include "chrome/browser/extensions/extension_tab_util.h"
+#include "chrome/browser/ui/extensions/extension_side_panel_utils.h"
+#include "extensions/browser/extension_registry.h"
 #include "chrome/browser/ui/autofill/address_bubbles_icon_controller.h"
 #include "chrome/browser/ui/autofill/autofill_bubble_base.h"
 #include "chrome/browser/ui/autofill/payments/mandatory_reauth_bubble_controller_impl.h"
@@ -253,6 +258,37 @@ void BrowserActions::InitializeBrowserActions() {
             .Build());
   }
 
+  // Add third-party LLM panel if feature is enabled
+  if (base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel)) {
+    root_action_item_->AddChild(
+        SidePanelAction(SidePanelEntryId::kThirdPartyLlm,
+                        IDS_THIRD_PARTY_LLM_TITLE,
+                        IDS_THIRD_PARTY_LLM_TITLE,
+                        vector_icons::kChatOrangeIcon,
+                        kActionSidePanelShowThirdPartyLlm, bwi, true)
+            .Build());
+  }
+
+  // Add Clash of GPTs action if feature is enabled
+  if (base::FeatureList::IsEnabled(features::kClashOfGpts)) {
+    root_action_item_->AddChild(
+        ChromeMenuAction(
+            base::BindRepeating(
+                [](BrowserWindowInterface* bwi, actions::ActionItem* item,
+                   actions::ActionInvocationContext context) {
+                  if (auto* browser_view = BrowserView::GetBrowserViewForBrowser(bwi)) {
+                    chrome::ExecuteCommand(browser_view->browser(), IDC_OPEN_CLASH_OF_GPTS);
+                  }
+                },
+                bwi),
+            kActionSidePanelShowClashOfGpts,
+            IDS_CLASH_OF_GPTS_TITLE,
+            IDS_CLASH_OF_GPTS_TOOLTIP,
+            vector_icons::kClashOfGptsIcon)
+            .Build());
+  }
+
+
   if (HistorySidePanelCoordinator::IsSupported()) {
     root_action_item_->AddChild(
         SidePanelAction(SidePanelEntryId::kHistory, IDS_HISTORY_TITLE,
