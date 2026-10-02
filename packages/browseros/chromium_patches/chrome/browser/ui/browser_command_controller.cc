diff --git a/chrome/browser/ui/browser_command_controller.cc b/chrome/browser/ui/browser_command_controller.cc
index 3741ba7671d225e307800a48c23c4c4bd1fd121c..f1b100f99c11c3821456157bcdeec51163e388c5 100644
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
@@ -27,11 +29,14 @@
 #include "chrome/browser/actor/ui/actor_overlay_web_view.h"
 #include "chrome/browser/bookmarks/bookmark_model_factory.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/browsing_data/browsing_data_important_sites_util.h"
 #include "chrome/browser/defaults.h"
 #include "chrome/browser/devtools/devtools_window.h"
 #include "chrome/browser/devtools/features.h"
 #include "chrome/browser/enterprise/isolated_mode/isolated_mode_settings_service_factory.h"
+#include "chrome/browser/extensions/api/side_panel/side_panel_service.h"
+#include "chrome/browser/extensions/extension_tab_util.h"
 #include "chrome/browser/feedback/public/feedback_source.h"
 #include "chrome/browser/feedback/show_feedback_page.h"
 #include "chrome/browser/glic/glic_enums.h"
@@ -40,6 +45,7 @@
 #include "chrome/browser/glic/public/glic_enabling.h"
 #include "chrome/browser/glic/public/glic_keyed_service_factory.h"
 #include "chrome/browser/glic/public/service/glic_instance_coordinator.h"
+#include "chrome/browser/infobars/simple_alert_infobar_creator.h"
 #include "chrome/browser/lifetime/application_lifetime.h"
 #include "chrome/browser/prefs/incognito_mode_prefs.h"
 #include "chrome/browser/profiles/profile.h"
@@ -78,6 +84,7 @@
 #include "chrome/browser/ui/read_anything/read_anything_controller.h"
 #include "chrome/browser/ui/read_anything/read_anything_entry_point_controller.h"
 #include "chrome/browser/ui/side_panel/side_panel_entry_id.h"
+#include "chrome/browser/ui/side_panel/side_panel_entry_key.h"
 #include "chrome/browser/ui/side_panel/side_panel_ui.h"
 #include "chrome/browser/ui/singleton_tabs.h"
 #include "chrome/browser/ui/startup/default_browser_prompt/default_browser_prompt_manager.h"
@@ -93,6 +100,7 @@
 #include "chrome/browser/ui/ui_features.h"
 #include "chrome/browser/ui/views/frame/browser_view.h"
 #include "chrome/browser/ui/views/side_panel/tabs_from_other_devices/tabs_from_other_devices_side_panel_coordinator.h"
+#include "chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.h"
 #include "chrome/browser/ui/web_applications/app_browser_controller.h"
 #include "chrome/browser/ui/web_applications/web_app_dialog_utils.h"
 #include "chrome/browser/ui/web_applications/web_app_launch_utils.h"
@@ -113,6 +121,7 @@
 #include "components/bookmarks/common/bookmark_bar_visibility_state.h"
 #include "components/bookmarks/common/bookmark_pref_names.h"
 #include "components/dom_distiller/core/dom_distiller_features.h"
+#include "components/infobars/content/content_infobar_manager.h"
 #include "components/input/native_web_keyboard_event.h"
 #include "components/lens/buildflags.h"
 #include "components/password_manager/core/browser/manage_passwords_referrer.h"
@@ -579,7 +588,6 @@ void BrowserCommandController::GlicActiveInstanceChanged(
   UpdateGlicState();
 }
 
-
 void BrowserCommandController::FindBarVisibilityChanged() {
   // Block find command updates in locked fullscreen mode unless the instance is
   // locked for OnTask (only relevant for non-web browser scenarios).
@@ -1248,6 +1256,64 @@ void BrowserCommandController::HandleCommandWithDisposition(
       SidePanelUI::From(browser_)->Show(SidePanelEntryId::kBookmarks,
                                         SidePanelOpenTrigger::kAppMenu);
       break;
+    case IDC_SHOW_THIRD_PARTY_LLM_SIDE_PANEL:
+      if (base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel)) {
+        SidePanelUI::From(browser_)->Toggle(
+            SidePanelEntryKey(SidePanelEntryId::kThirdPartyLlm),
+            SidePanelOpenTrigger::kAppMenu);
+      }
+      break;
+    case IDC_CYCLE_THIRD_PARTY_LLM_PROVIDER:
+      if (base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel)) {
+        if (ThirdPartyLlmPanelCoordinator* coordinator =
+                browser_->GetFeatures().third_party_llm_panel_coordinator()) {
+          coordinator->CycleProvider();
+        }
+      }
+      break;
+    case IDC_TOGGLE_BROWSEROS_AGENT: {
+      if (!browseros::IsActiveBrowserOSExtension(
+              browseros::kAgentExtensionId)) {
+        break;
+      }
+      content::WebContents* active_contents =
+          browser_->tab_strip_model()->GetActiveWebContents();
+      if (!active_contents) {
+        break;
+      }
+      int tab_id = extensions::ExtensionTabUtil::GetTabId(active_contents);
+      Profile* profile = browser_->GetProfile();
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
+              u"BrowserOS Agent is installing/updating. Please try again "
+              u"shortly.",
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
@@ -2013,6 +2079,15 @@ void BrowserCommandController::InitCommandState() {
   }
 
   command_updater_->UpdateCommandEnabled(IDC_SHOW_BOOKMARK_SIDE_PANEL, true);
+  command_updater_->UpdateCommandEnabled(
+      IDC_SHOW_THIRD_PARTY_LLM_SIDE_PANEL,
+      base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel));
+  command_updater_->UpdateCommandEnabled(
+      IDC_CYCLE_THIRD_PARTY_LLM_PROVIDER,
+      base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel));
+  command_updater_->UpdateCommandEnabled(
+      IDC_TOGGLE_BROWSEROS_AGENT,
+      browseros::IsActiveBrowserOSExtension(browseros::kAgentExtensionId));
 
   if (browser_->GetType() == BrowserWindowInterface::Type::TYPE_NORMAL) {
     // Reading list commands.
