diff --git a/chrome/browser/ui/side_panel/side_panel_entry_id.h b/chrome/browser/ui/side_panel/side_panel_entry_id.h
index f77bd9c8db7dbf58b7572ebf1c4c1b714315ae42..2318e896f9e57764763fc19c0da9c106170a5ff4 100644
--- a/chrome/browser/ui/side_panel/side_panel_entry_id.h
+++ b/chrome/browser/ui/side_panel/side_panel_entry_id.h
@@ -43,6 +43,7 @@
   V(kTabsFromOtherDevices, kActionSidePanelShowTabsFromOtherDevices,          \
     "TabsFromOtherDevices")                                                   \
   V(kSidePanelDev, std::nullopt, "SidePanelDev")                              \
+  V(kThirdPartyLlm, kActionSidePanelShowThirdPartyLlm, "ThirdPartyLlm")       \
   /* Extensions (nothing more should be added below here) */                  \
   V(kExtension, std::nullopt, "Extension")
 
