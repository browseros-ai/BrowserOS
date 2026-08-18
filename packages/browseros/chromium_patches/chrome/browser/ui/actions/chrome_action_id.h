diff --git a/chrome/browser/ui/actions/chrome_action_id.h b/chrome/browser/ui/actions/chrome_action_id.h
index 1a6acad48b6ffcc49e77229d79ffb838681abfa8..a2b7ebd4fa34109e9165a0050af13d8406e384d0 100644
--- a/chrome/browser/ui/actions/chrome_action_id.h
+++ b/chrome/browser/ui/actions/chrome_action_id.h
@@ -573,7 +573,9 @@
   E(kActionSidePanelShowSideSearch) \
   E(kActionSidePanelShowMerchantTrust) \
   E(kActionSidePanelShowTabsFromOtherDevices, \
-    IDC_SHOW_TABS_FROM_OTHER_DEVICES_SIDE_PANEL)
+    IDC_SHOW_TABS_FROM_OTHER_DEVICES_SIDE_PANEL) \
+  E(kActionSidePanelShowThirdPartyLlm) \
+  E(kActionBrowserOSAgent)
 
 #define TOOLBAR_PINNABLE_ACTION_IDS \
   E(kActionHome, IDC_HOME) \
