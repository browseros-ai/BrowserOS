diff --git a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler.cc b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler.cc
index 31b360347f2bc..1192c83265e9a 100644
--- a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler.cc
+++ b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler.cc
@@ -92,6 +92,11 @@ MojoActionForChromeAction(actions::ActionId action_id) {
       return side_panel::customize_chrome::mojom::ActionId::kSplitTab;
     case kActionSidePanelShowContextualTasks:
       return side_panel::customize_chrome::mojom::ActionId::kContextualTasks;
+    // BrowserOS: custom toolbar actions
+    case kActionSidePanelShowThirdPartyLlm:
+      return side_panel::customize_chrome::mojom::ActionId::kShowThirdPartyLlm;
+    case kActionSidePanelShowClashOfGpts:
+      return side_panel::customize_chrome::mojom::ActionId::kShowClashOfGpts;
     default:
       return std::nullopt;
   }
@@ -152,6 +157,11 @@ std::optional<actions::ActionId> ChromeActionForMojoAction(
       return kActionSplitTab;
     case side_panel::customize_chrome::mojom::ActionId::kContextualTasks:
       return kActionSidePanelShowContextualTasks;
+    // BrowserOS: custom toolbar actions
+    case side_panel::customize_chrome::mojom::ActionId::kShowThirdPartyLlm:
+      return kActionSidePanelShowThirdPartyLlm;
+    case side_panel::customize_chrome::mojom::ActionId::kShowClashOfGpts:
+      return kActionSidePanelShowClashOfGpts;
     default:
       return std::nullopt;
   }
@@ -342,6 +352,10 @@ void CustomizeToolbarHandler::ListActions(ListActionsCallback callback) {
              side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
   add_action(kActionSidePanelShowReadingList,
              side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
+  add_action(kActionSidePanelShowThirdPartyLlm,
+             side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
+  add_action(kActionSidePanelShowClashOfGpts,
+             side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
   add_action(kActionSidePanelShowHistoryCluster,
              side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
   add_action(kActionShowDownloads,
