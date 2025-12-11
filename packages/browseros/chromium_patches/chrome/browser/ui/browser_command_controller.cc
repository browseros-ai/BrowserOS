diff --git a/chrome/browser/ui/browser_command_controller.cc b/chrome/browser/ui/browser_command_controller.cc
index deb531f8832e3..78591a95b2bc7 100644
--- a/chrome/browser/ui/browser_command_controller.cc
+++ b/chrome/browser/ui/browser_command_controller.cc
@@ -70,6 +70,8 @@
 #include "chrome/browser/ui/ui_features.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_entry_id.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_ui.h"
+#include "chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.h"
+#include "chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_coordinator.h"
 #include "chrome/browser/ui/web_applications/app_browser_controller.h"
 #include "chrome/browser/ui/web_applications/web_app_dialog_utils.h"
 #include "chrome/browser/ui/web_applications/web_app_launch_utils.h"
@@ -104,7 +106,13 @@
 #include "content/public/browser/web_contents_observer.h"
 #include "content/public/common/profiling.h"
 #include "content/public/common/url_constants.h"
+#include "chrome/browser/extensions/api/side_panel/side_panel_service.h"
+#include "chrome/browser/extensions/browseros_extension_constants.h"
+#include "chrome/browser/extensions/extension_tab_util.h"
+#include "chrome/browser/infobars/simple_alert_infobar_creator.h"
+#include "components/infobars/content/content_infobar_manager.h"
 #include "extensions/browser/extension_registrar.h"
+#include "extensions/browser/extension_registry.h"
 #include "extensions/common/extension_urls.h"
 #include "printing/buildflags/buildflags.h"
 #include "ui/actions/actions.h"
@@ -988,6 +996,71 @@ bool BrowserCommandController::ExecuteCommandWithDisposition(
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
+    case IDC_OPEN_CLASH_OF_GPTS:
+      if (base::FeatureList::IsEnabled(features::kClashOfGpts)) {
+        ClashOfGptsCoordinator* coordinator =
+            browser_->browser_window_features()->clash_of_gpts_coordinator();
+        // If not showing properly, close and recreate
+        if (!coordinator->IsShowing()) {
+          coordinator->Close();
+        }
+        coordinator->Show();
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
+              .GetByID(extensions::browseros::kAgentV2ExtensionId);
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
@@ -1648,6 +1721,13 @@ void BrowserCommandController::InitCommandState() {
   }
 
   command_updater_.UpdateCommandEnabled(IDC_SHOW_BOOKMARK_SIDE_PANEL, true);
+  command_updater_.UpdateCommandEnabled(IDC_SHOW_THIRD_PARTY_LLM_SIDE_PANEL,
+                                        base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel));
+  command_updater_.UpdateCommandEnabled(IDC_CYCLE_THIRD_PARTY_LLM_PROVIDER,
+                                        base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel));
+  command_updater_.UpdateCommandEnabled(IDC_OPEN_CLASH_OF_GPTS,
+                                        base::FeatureList::IsEnabled(features::kClashOfGpts));
+  command_updater_.UpdateCommandEnabled(IDC_TOGGLE_BROWSEROS_AGENT, true);
 
   if (browser_->is_type_normal()) {
     // Reading list commands.
