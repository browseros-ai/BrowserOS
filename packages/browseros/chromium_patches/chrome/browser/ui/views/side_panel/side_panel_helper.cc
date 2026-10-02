diff --git a/chrome/browser/ui/views/side_panel/side_panel_helper.cc b/chrome/browser/ui/views/side_panel/side_panel_helper.cc
index e1e7ccb48cdf3e0baaee33fad31ebcb669110435..1cae6c2ed8d5f97614189d5e9446c38d8729a36b 100644
--- a/chrome/browser/ui/views/side_panel/side_panel_helper.cc
+++ b/chrome/browser/ui/views/side_panel/side_panel_helper.cc
@@ -18,6 +18,7 @@
 #include "chrome/browser/ui/views/side_panel/history_clusters/history_clusters_side_panel_coordinator.h"
 #include "chrome/browser/ui/views/side_panel/reading_list/reading_list_side_panel_coordinator.h"
 #include "chrome/browser/ui/views/side_panel/tabs_from_other_devices/tabs_from_other_devices_side_panel_coordinator.h"
+#include "chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.h"
 #include "chrome/browser/ui/webui_browser/webui_browser.h"
 #include "components/history_clusters/core/features.h"
 #include "components/history_clusters/core/history_clusters_service.h"
@@ -48,6 +49,13 @@ void SidePanelHelper::PopulateGlobalEntries(
         ->CreateAndRegisterEntry(window_registry);
   }
 
+  // Add third-party LLM panel.
+  if (base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel)) {
+    browser->GetFeatures()
+        .third_party_llm_panel_coordinator()
+        ->CreateAndRegisterEntry(window_registry);
+  }
+
   // Add history clusters.
   if (HistoryClustersSidePanelCoordinator::IsSupported(browser->GetProfile()) &&
       !HistorySidePanelCoordinator::IsSupported()) {
