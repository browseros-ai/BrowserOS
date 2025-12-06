diff --git a/chrome/browser/extensions/api/side_panel/side_panel_api.cc b/chrome/browser/extensions/api/side_panel/side_panel_api.cc
index f89e8095cc91f..90e334e435f2a 100644
--- a/chrome/browser/extensions/api/side_panel/side_panel_api.cc
+++ b/chrome/browser/extensions/api/side_panel/side_panel_api.cc
@@ -71,11 +71,11 @@ ExtensionFunction::ResponseAction SidePanelOpenFunction::RunFunction() {
   EXTENSION_FUNCTION_VALIDATE(extension());
 
   // `sidePanel.open()` requires a user gesture.
-  if (!user_gesture()) {
-    return RespondNow(
-        Error("`sidePanel.open()` may only be called in "
-              "response to a user gesture."));
-  }
+  // if (!user_gesture()) {
+  //   return RespondNow(
+  //       Error("`sidePanel.open()` may only be called in "
+  //             "response to a user gesture."));
+  // }
 
   std::optional<api::side_panel::Open::Params> params =
       api::side_panel::Open::Params::Create(args());
@@ -152,4 +152,54 @@ ExtensionFunction::ResponseAction SidePanelCloseFunction::RunFunction() {
   return RespondNow(NoArguments());
 }
 
+ExtensionFunction::ResponseAction
+SidePanelBrowserosToggleFunction::RunFunction() {
+  EXTENSION_FUNCTION_VALIDATE(extension());
+
+  std::optional<api::side_panel::BrowserosToggle::Params> params =
+      api::side_panel::BrowserosToggle::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  // Convert optional open parameter.
+  std::optional<bool> desired_state = std::nullopt;
+  if (params->options.open.has_value()) {
+    desired_state = params->options.open.value();
+  }
+
+  SidePanelService* service = GetService();
+  base::expected<bool, std::string> toggle_result =
+      service->BrowserosToggleSidePanelForTab(
+          *extension(), browser_context(), params->options.tab_id,
+          include_incognito_information(), desired_state);
+
+  if (!toggle_result.has_value()) {
+    return RespondNow(Error(std::move(toggle_result.error())));
+  }
+
+  api::side_panel::BrowserosToggleResult result;
+  result.opened = toggle_result.value();
+  return RespondNow(WithArguments(result.ToValue()));
+}
+
+ExtensionFunction::ResponseAction
+SidePanelBrowserosIsOpenFunction::RunFunction() {
+  EXTENSION_FUNCTION_VALIDATE(extension());
+
+  std::optional<api::side_panel::BrowserosIsOpen::Params> params =
+      api::side_panel::BrowserosIsOpen::Params::Create(args());
+  EXTENSION_FUNCTION_VALIDATE(params);
+
+  SidePanelService* service = GetService();
+  base::expected<bool, std::string> is_open_result =
+      service->BrowserosIsSidePanelOpenForTab(
+          *extension(), browser_context(), params->options.tab_id,
+          include_incognito_information());
+
+  if (!is_open_result.has_value()) {
+    return RespondNow(Error(std::move(is_open_result.error())));
+  }
+
+  return RespondNow(WithArguments(is_open_result.value()));
+}
+
 }  // namespace extensions
