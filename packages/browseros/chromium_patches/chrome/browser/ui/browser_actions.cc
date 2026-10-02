diff --git a/chrome/browser/ui/browser_actions.cc b/chrome/browser/ui/browser_actions.cc
index 83b50c9d0440e75e275d5a09aca78a0af8dd850b..6762a8d686fbbd10aaf496480272ffc2c8b611f8 100644
--- a/chrome/browser/ui/browser_actions.cc
+++ b/chrome/browser/ui/browser_actions.cc
@@ -21,6 +21,7 @@
 #include "build/branding_buildflags.h"
 #include "chrome/app/chrome_command_ids.h"
 #include "chrome/app/vector_icons/vector_icons.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/browsing_data/browsing_data_important_sites_util.h"
 #include "chrome/browser/contextual_cueing/contextual_cueing_controller.h"
 #include "chrome/browser/contextual_cueing/features.h"
@@ -29,6 +30,8 @@
 #include "chrome/browser/contextual_tasks/entry_point_eligibility_manager.h"
 #include "chrome/browser/devtools/devtools_window.h"
 #include "chrome/browser/enterprise/isolated_mode/isolated_mode_settings_service_factory.h"
+#include "chrome/browser/extensions/api/side_panel/side_panel_service.h"
+#include "chrome/browser/extensions/extension_tab_util.h"
 #include "chrome/browser/glic/browser_ui/glic_vector_icon_manager.h"
 #include "chrome/browser/glic/glic_pref_names.h"
 #include "chrome/browser/glic/host/glic.mojom.h"
@@ -36,6 +39,7 @@
 #include "chrome/browser/glic/public/glic_keyed_service.h"
 #include "chrome/browser/glic/resources/grit/glic_browser_resources.h"
 #include "chrome/browser/indigo/indigo_page_action_controller.h"
+#include "chrome/browser/infobars/simple_alert_infobar_creator.h"
 #include "chrome/browser/lifetime/application_lifetime.h"
 #include "chrome/browser/media/router/media_router_feature.h"
 #include "chrome/browser/prefs/incognito_mode_prefs.h"
@@ -115,6 +119,7 @@
 #include "chrome/browser/ui/commerce/commerce_ui_tab_helper.h"
 #include "chrome/browser/ui/customize_chrome/side_panel_controller.h"
 #include "chrome/browser/ui/dialogs/browser_dialogs.h"
+#include "chrome/browser/ui/extensions/extension_side_panel_utils.h"
 #include "chrome/browser/ui/intent_picker_tab_helper.h"
 #include "chrome/browser/ui/lens/lens_overlay_controller.h"
 #include "chrome/browser/ui/lens/lens_overlay_entry_point_controller.h"
@@ -189,6 +194,7 @@
 #include "chrome/common/url_constants.h"
 #include "chrome/grit/branded_strings.h"
 #include "chrome/grit/generated_resources.h"
+#include "chrome/grit/theme_resources.h"
 #include "components/autofill/core/common/autofill_features.h"
 #include "components/autofill/core/common/autofill_payments_features.h"
 #include "components/bookmarks/common/bookmark_bar_visibility_state.h"
@@ -198,6 +204,7 @@
 #include "components/content_settings/core/common/features.h"
 #include "components/contextual_tasks/public/features.h"
 #include "components/feature_engagement/public/feature_constants.h"
+#include "components/infobars/content/content_infobar_manager.h"
 #include "components/lens/lens_features.h"
 #include "components/lens/lens_overlay_invocation_source.h"
 #include "components/media_router/browser/media_router_dialog_controller.h"
@@ -231,6 +238,7 @@
 #include "components/lens/lens_overlay_invocation_source.h"
 #include "components/user_prefs/user_prefs.h"
 #include "components/vector_icons/vector_icons.h"
+#include "extensions/browser/extension_registry.h"
 #include "printing/buildflags/buildflags.h"
 #include "ui/accessibility/accessibility_features.h"
 #include "ui/actions/actions.h"
@@ -459,6 +467,93 @@ void BrowserActions::InitializeSidePanelActions() {
             .Build());
   }
 
+  // The window coordinator registers the matching entry; the action keeps
+  // the same feature gate so a disabled panel cannot be opened or pinned.
+  if (base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel)) {
+    root_action_item_->AddChild(
+        SidePanelAction(SidePanelEntryId::kThirdPartyLlm,
+                        IDS_THIRD_PARTY_LLM_TITLE, IDS_THIRD_PARTY_LLM_TITLE,
+                        vector_icons::kChatOrangeIcon,
+                        kActionSidePanelShowThirdPartyLlm, bwi, true)
+            .Build());
+  }
+
+  if (browseros::IsActiveBrowserOSExtension(browseros::kAgentExtensionId)) {
+    root_action_item_->AddChild(
+        actions::ActionItem::Builder(
+            base::BindRepeating(
+                [](BrowserWindowInterface* bwi, actions::ActionItem* item,
+                   actions::ActionInvocationContext context) {
+                  auto* tab = bwi->GetActiveTabInterface();
+                  if (!tab || !tab->GetContents()) {
+                    LOG(WARNING) << "browseros: No active tab for Agent action";
+                    return;
+                  }
+
+                  content::WebContents* contents = tab->GetContents();
+                  Profile* profile = Profile::FromBrowserContext(
+                      contents->GetBrowserContext());
+
+                  const extensions::Extension* extension =
+                      extensions::ExtensionRegistry::Get(profile)
+                          ->enabled_extensions()
+                          .GetByID(browseros::kAgentExtensionId);
+                  if (!extension) {
+                    LOG(WARNING) << "browseros: Agent extension not found";
+                    infobars::ContentInfoBarManager* infobar_manager =
+                        infobars::ContentInfoBarManager::FromWebContents(
+                            contents);
+                    if (infobar_manager) {
+                      CreateSimpleAlertInfoBar(
+                          infobar_manager,
+                          infobars::InfoBarDelegate::
+                              BROWSEROS_AGENT_INSTALLING_INFOBAR_DELEGATE,
+                          nullptr,
+                          u"BrowserOS Agent is installing/updating. Please try "
+                          u"again shortly.",
+                          /*auto_expire=*/true,
+                          /*should_animate=*/true,
+                          /*closeable=*/true);
+                    }
+                    return;
+                  }
+
+                  int tab_id = extensions::ExtensionTabUtil::GetTabId(contents);
+                  LOG(INFO) << "browseros: Agent toolbar action for tab_id="
+                            << tab_id;
+
+                  extensions::SidePanelService* service =
+                      extensions::SidePanelService::Get(profile);
+                  if (!service) {
+                    LOG(WARNING) << "browseros: SidePanelService not found";
+                    return;
+                  }
+
+                  auto result = service->BrowserosToggleSidePanelForTab(
+                      *extension, profile, tab_id,
+                      /*include_incognito_information=*/true,
+                      /*desired_state=*/std::nullopt);
+
+                  if (!result.has_value()) {
+                    LOG(WARNING)
+                        << "browseros: Agent toggle failed: " << result.error();
+                  } else {
+                    LOG(INFO)
+                        << "browseros: Agent toggle result: " << result.value();
+                  }
+                },
+                bwi))
+            .SetActionId(kActionBrowserOSAgent)
+            .SetText(u"Assistant")
+            .SetTooltipText(u"Ask BrowserOS")
+            .SetImage(ui::ImageModel::FromResourceId(IDR_PRODUCT_LOGO_16))
+            .SetProperty(
+                actions::kActionItemPinnableKey,
+                std::underlying_type_t<actions::ActionPinnableState>(
+                    actions::ActionPinnableState::kEnterpriseControlled))
+            .Build());
+  }
+
   if (HistorySidePanelCoordinator::IsSupported()) {
     root_action_item_->AddChild(
         SidePanelAction(SidePanelEntryId::kHistory, IDS_HISTORY_TITLE,
