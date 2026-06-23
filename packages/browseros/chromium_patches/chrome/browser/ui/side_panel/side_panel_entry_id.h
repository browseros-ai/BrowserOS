diff --git a/chrome/browser/ui/side_panel/side_panel_entry_id.h b/chrome/browser/ui/side_panel/side_panel_entry_id.h
index b089b5e276bab..437fa04663b0e 100644
--- a/chrome/browser/ui/side_panel/side_panel_entry_id.h
+++ b/chrome/browser/ui/side_panel/side_panel_entry_id.h
@@ -42,6 +42,7 @@
   V(kGlic, kActionSidePanelShowGlic, "Glic")                                  \
   V(kTabsFromOtherDevices, kActionSidePanelShowTabsFromOtherDevices,          \
     "TabsFromOtherDevices")                                                   \
+  V(kThirdPartyLlm, kActionSidePanelShowThirdPartyLlm, "ThirdPartyLlm")       \
   /* Extensions (nothing more should be added below here) */                  \
   V(kExtension, std::nullopt, "Extension")
 
