diff --git a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler.cc b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler.cc
index 96695a255df870dd5f5109bd00ec0a0f3c5edd02..e236bca52ff994d0a112693b0637e4fa2237fac2 100644
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
@@ -262,7 +268,6 @@ void CustomizeToolbarHandler::ListActions(ListActionsCallback callback) {
 
   actions.push_back(std::move(split_tab_action));
 
-
   const auto add_action =
       [&actions, this, &provider, scale_factor, bwi](
           actions::ActionId id,
@@ -333,6 +338,8 @@ void CustomizeToolbarHandler::ListActions(ListActionsCallback callback) {
              side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
   add_action(kActionSidePanelShowTabsFromOtherDevices,
              side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
+  add_action(kActionSidePanelShowThirdPartyLlm,
+             side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
   add_action(kActionSidePanelShowHistoryCluster,
              side_panel::customize_chrome::mojom::CategoryId::kYourChrome);
   add_action(kActionShowDownloads,
