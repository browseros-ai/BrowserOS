diff --git a/chrome/browser/ui/actions/chrome_action_id.h b/chrome/browser/ui/actions/chrome_action_id.h
index ccd08503f9a87..cc4970640ca12 100644
--- a/chrome/browser/ui/actions/chrome_action_id.h
+++ b/chrome/browser/ui/actions/chrome_action_id.h
@@ -553,7 +553,10 @@
   E(kActionSidePanelShowShoppingInsights) \
   E(kActionSidePanelShowSideSearch) \
   E(kActionSidePanelShowUserNote) \
-  E(kActionSidePanelShowMerchantTrust)
+  E(kActionSidePanelShowMerchantTrust) \
+  E(kActionSidePanelShowThirdPartyLlm) \
+  E(kActionSidePanelShowClashOfGpts) \
+  E(kActionBrowserOSAgent)
 
 #define TOOLBAR_PINNABLE_ACTION_IDS \
   E(kActionHome, IDC_HOME) \
