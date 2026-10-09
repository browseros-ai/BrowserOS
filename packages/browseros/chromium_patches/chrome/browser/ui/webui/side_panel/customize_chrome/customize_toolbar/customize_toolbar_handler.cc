diff --git a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler.cc b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler.cc
index 7f51aadbe7ca492a928817a016de74bb9a2cc300..996656fae4a7d487eb0ef0f335abcedee11c07fc 100644
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
@@ -332,6 +338,8 @@ void CustomizeToolbarHandler::ListActions(ListActionsCallback callback) {
              side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
   add_action(kActionSidePanelShowTabsFromOtherDevices,
              side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
+  add_action(kActionSidePanelShowThirdPartyLlm,
+             side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
   add_action(kActionSidePanelShowHistoryCluster,
              side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
   add_action(kActionShowDownloads,
