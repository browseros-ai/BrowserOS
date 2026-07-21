diff --git a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler.cc b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler.cc
index 96695a255df870dd5f5109bd00ec0a0f3c5edd02..1560e97a5d65c1f96e0b3412a0ec28670eea8d1b 100644
--- a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler.cc
+++ b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler.cc
@@ -98,6 +98,9 @@ MojoActionForChromeAction(actions::ActionId action_id) {
     case kActionSidePanelShowTabsFromOtherDevices:
       return side_panel::customize_chrome::mojom::ActionId::
           kShowTabsFromOtherDevices;
+    // BrowserOS: custom toolbar actions
+    case kActionSidePanelShowThirdPartyLlm:
+      return side_panel::customize_chrome::mojom::ActionId::kShowThirdPartyLlm;
     default:
       return std::nullopt;
   }
@@ -161,6 +164,9 @@ std::optional<actions::ActionId> ChromeActionForMojoAction(
       return kActionSplitTab;
     case side_panel::customize_chrome::mojom::ActionId::kContextualTasks:
       return kActionSidePanelShowContextualTasks;
+    // BrowserOS: custom toolbar actions
+    case side_panel::customize_chrome::mojom::ActionId::kShowThirdPartyLlm:
+      return kActionSidePanelShowThirdPartyLlm;
     default:
       return std::nullopt;
   }
@@ -333,6 +339,8 @@ void CustomizeToolbarHandler::ListActions(ListActionsCallback callback) {
              side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
   add_action(kActionSidePanelShowTabsFromOtherDevices,
              side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
+  add_action(kActionSidePanelShowThirdPartyLlm,
+             side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
   add_action(kActionSidePanelShowHistoryCluster,
              side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
   add_action(kActionShowDownloads,
