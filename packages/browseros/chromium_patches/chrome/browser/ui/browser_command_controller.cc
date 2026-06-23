diff --git a/chrome/browser/ui/browser_command_controller.cc b/chrome/browser/ui/browser_command_controller.cc
index 738696abf04fa..aa1a1a1ce6a85 100644
--- a/chrome/browser/ui/browser_command_controller.cc
+++ b/chrome/browser/ui/browser_command_controller.cc
@@ -7,7 +7,9 @@
 #include <stddef.h>
 
 #include <algorithm>
+#include <optional>
 #include <string>
+#include <tuple>
 
 #include "base/check_deref.h"
 #include "base/command_line.h"
@@ -23,11 +25,14 @@
 #include "build/build_config.h"
 #include "chrome/app/chrome_command_ids.h"
 #include "chrome/browser/actor/ui/actor_overlay_web_view.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/browser_process.h"
 #include "chrome/browser/browsing_data/browsing_data_important_sites_util.h"
 #include "chrome/browser/defaults.h"
 #include "chrome/browser/devtools/devtools_window.h"
 #include "chrome/browser/devtools/features.h"
+#include "chrome/browser/extensions/api/side_panel/side_panel_service.h"
+#include "chrome/browser/extensions/extension_tab_util.h"
 #include "chrome/browser/feedback/public/feedback_source.h"
 #include "chrome/browser/feedback/show_feedback_page.h"
 #include "chrome/browser/glic/fre/glic_fre_controller.h"
@@ -37,6 +42,7 @@
 #include "chrome/browser/glic/public/glic_enabling.h"
 #include "chrome/browser/glic/public/glic_keyed_service_factory.h"
 #include "chrome/browser/glic/public/service/glic_instance_coordinator.h"
+#include "chrome/browser/infobars/simple_alert_infobar_creator.h"
 #include "chrome/browser/lifetime/application_lifetime.h"
 #include "chrome/browser/prefs/incognito_mode_prefs.h"
 #include "chrome/browser/profiles/profile.h"
@@ -86,6 +92,7 @@
 #include "chrome/browser/ui/toolbar/chrome_labs/chrome_labs_utils.h"
 #include "chrome/browser/ui/ui_features.h"
 #include "chrome/browser/ui/views/frame/browser_view.h"
+#include "chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.h"
 #include "chrome/browser/ui/web_applications/app_browser_controller.h"
 #include "chrome/browser/ui/web_applications/web_app_dialog_utils.h"
 #include "chrome/browser/ui/web_applications/web_app_launch_utils.h"
@@ -120,6 +127,7 @@
 #include "content/public/browser/web_contents_observer.h"
 #include "content/public/common/profiling.h"
 #include "content/public/common/url_constants.h"
+#include "components/infobars/content/content_infobar_manager.h"
 #include "extensions/browser/extension_registrar.h"
 #include "extensions/browser/extension_registry.h"
 #include "extensions/common/extension_urls.h"
@@ -1089,6 +1097,60 @@ bool BrowserCommandController::ExecuteCommandWithDisposition(
       browser_->GetFeatures().side_panel_ui()->Show(
           SidePanelEntryId::kBookmarks, SidePanelOpenTrigger::kAppMenu);
       break;
+    case IDC_SHOW_THIRD_PARTY_LLM_SIDE_PANEL:
+      if (base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel)) {
+        browser_->GetFeatures().side_panel_ui()->Toggle(
+            SidePanelEntry::Key(SidePanelEntryId::kThirdPartyLlm),
+            SidePanelOpenTrigger::kAppMenu);
+      }
+      break;
+    case IDC_CYCLE_THIRD_PARTY_LLM_PROVIDER:
+      if (base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel)) {
+        if (ThirdPartyLlmPanelCoordinator* coordinator =
+                browser_->browser_window_features()
+                    ->third_party_llm_panel_coordinator()) {
+          coordinator->CycleProvider();
+        }
+      }
+      break;
+    case IDC_TOGGLE_BROWSEROS_AGENT: {
+      content::WebContents* active_contents =
+          browser_->tab_strip_model()->GetActiveWebContents();
+      if (!active_contents) {
+        break;
+      }
+      int tab_id = extensions::ExtensionTabUtil::GetTabId(active_contents);
+      Profile* profile = browser_->profile();
+      const extensions::Extension* extension =
+          extensions::ExtensionRegistry::Get(profile)
+              ->enabled_extensions()
+              .GetByID(browseros::kAgentExtensionId);
+      if (!extension) {
+        infobars::ContentInfoBarManager* infobar_manager =
+            infobars::ContentInfoBarManager::FromWebContents(active_contents);
+        if (infobar_manager) {
+          CreateSimpleAlertInfoBar(
+              infobar_manager,
+              infobars::InfoBarDelegate::
+                  BROWSEROS_AGENT_INSTALLING_INFOBAR_DELEGATE,
+              nullptr,
+              u"BrowserOS Agent is installing/updating. Please try again shortly.",
+              /*auto_expire=*/true,
+              /*should_animate=*/true,
+              /*closeable=*/true);
+        }
+        break;
+      }
+      extensions::SidePanelService* service =
+          extensions::SidePanelService::Get(profile);
+      if (service) {
+        std::ignore = service->BrowserosToggleSidePanelForTab(
+            *extension, profile, tab_id,
+            /*include_incognito_information=*/true,
+            /*desired_state=*/std::nullopt);
+      }
+      break;
+    }
     case IDC_SHOW_APP_MENU:
       base::RecordAction(base::UserMetricsAction("Accel_Show_App_Menu"));
       ShowAppMenu(browser_);
@@ -1802,6 +1864,11 @@ void BrowserCommandController::InitCommandState() {
   }
 
   command_updater_.UpdateCommandEnabled(IDC_SHOW_BOOKMARK_SIDE_PANEL, true);
+  command_updater_.UpdateCommandEnabled(IDC_SHOW_THIRD_PARTY_LLM_SIDE_PANEL,
+                                        base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel));
+  command_updater_.UpdateCommandEnabled(IDC_CYCLE_THIRD_PARTY_LLM_PROVIDER,
+                                        base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel));
+  command_updater_.UpdateCommandEnabled(IDC_TOGGLE_BROWSEROS_AGENT, true);
 
   if (browser_->is_type_normal()) {
     // Reading list commands.
