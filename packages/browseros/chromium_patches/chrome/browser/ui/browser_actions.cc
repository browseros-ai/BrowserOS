diff --git a/chrome/browser/ui/browser_actions.cc b/chrome/browser/ui/browser_actions.cc
index fb3dba200be8c..9b7941ff32407 100644
--- a/chrome/browser/ui/browser_actions.cc
+++ b/chrome/browser/ui/browser_actions.cc
@@ -12,6 +12,7 @@
 #include "base/check_op.h"
 #include "base/functional/bind.h"
 #include "base/functional/callback_helpers.h"
+#include "chrome/grit/theme_resources.h"
 #include "chrome/app/vector_icons/vector_icons.h"
 #include "chrome/browser/devtools/devtools_window.h"
 #include "chrome/browser/prefs/incognito_mode_prefs.h"
@@ -20,6 +21,13 @@
 #include "chrome/browser/sharing_hub/sharing_hub_features.h"
 #include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/actions/chrome_actions.h"
+#include "chrome/browser/extensions/api/side_panel/side_panel_service.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
+#include "chrome/browser/extensions/extension_tab_util.h"
+#include "chrome/browser/infobars/simple_alert_infobar_creator.h"
+#include "components/infobars/content/content_infobar_manager.h"
+#include "chrome/browser/ui/extensions/extension_side_panel_utils.h"
+#include "extensions/browser/extension_registry.h"
 #include "chrome/browser/ui/autofill/address_bubbles_icon_controller.h"
 #include "chrome/browser/ui/autofill/autofill_bubble_base.h"
 #include "chrome/browser/ui/autofill/payments/mandatory_reauth_bubble_controller_impl.h"
@@ -253,6 +261,110 @@ void BrowserActions::InitializeBrowserActions() {
             .Build());
   }
 
+  // Add third-party LLM panel if feature is enabled
+  if (base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel)) {
+    root_action_item_->AddChild(
+        SidePanelAction(SidePanelEntryId::kThirdPartyLlm,
+                        IDS_THIRD_PARTY_LLM_TITLE,
+                        IDS_THIRD_PARTY_LLM_TITLE,
+                        vector_icons::kChatOrangeIcon,
+                        kActionSidePanelShowThirdPartyLlm, bwi, true)
+            .Build());
+  }
+
+  // Add Clash of GPTs action if feature is enabled
+  if (base::FeatureList::IsEnabled(features::kClashOfGpts)) {
+    root_action_item_->AddChild(
+        ChromeMenuAction(
+            base::BindRepeating(
+                [](BrowserWindowInterface* bwi, actions::ActionItem* item,
+                   actions::ActionInvocationContext context) {
+                  if (auto* browser_view = BrowserView::GetBrowserViewForBrowser(bwi)) {
+                    chrome::ExecuteCommand(browser_view->browser(), IDC_OPEN_CLASH_OF_GPTS);
+                  }
+                },
+                bwi),
+            kActionSidePanelShowClashOfGpts,
+            IDS_CLASH_OF_GPTS_TITLE,
+            IDS_CLASH_OF_GPTS_TOOLTIP,
+            vector_icons::kClashOfGptsIcon)
+            .Build());
+  }
+
+  // BrowserOS Agent - toggles contextual side panel on active tab.
+  // This is a native action that dynamically looks up the extension at
+  // invocation time, avoiding stale WeakPtr issues during extension updates.
+  root_action_item_->AddChild(
+      actions::ActionItem::Builder(
+          base::BindRepeating(
+              [](BrowserWindowInterface* bwi, actions::ActionItem* item,
+                 actions::ActionInvocationContext context) {
+                auto* tab = bwi->GetActiveTabInterface();
+                if (!tab || !tab->GetContents()) {
+                  LOG(WARNING) << "browseros: No active tab for Agent action";
+                  return;
+                }
+
+                content::WebContents* contents = tab->GetContents();
+                Profile* profile =
+                    Profile::FromBrowserContext(contents->GetBrowserContext());
+
+                const extensions::Extension* extension =
+                    extensions::ExtensionRegistry::Get(profile)
+                        ->enabled_extensions()
+                        .GetByID(browseros::kAgentV2ExtensionId);
+                if (!extension) {
+                  LOG(WARNING) << "browseros: Agent extension not found";
+                  infobars::ContentInfoBarManager* infobar_manager =
+                      infobars::ContentInfoBarManager::FromWebContents(contents);
+                  if (infobar_manager) {
+                    CreateSimpleAlertInfoBar(
+                        infobar_manager,
+                        infobars::InfoBarDelegate::
+                            BROWSEROS_AGENT_INSTALLING_INFOBAR_DELEGATE,
+                        nullptr,
+                        u"BrowserOS Agent is installing/updating. Please try again shortly.",
+                        /*auto_expire=*/true,
+                        /*should_animate=*/true,
+                        /*closeable=*/true);
+                  }
+                  return;
+                }
+
+                int tab_id = extensions::ExtensionTabUtil::GetTabId(contents);
+                LOG(INFO) << "browseros: Agent toolbar action for tab_id="
+                          << tab_id;
+
+                extensions::SidePanelService* service =
+                    extensions::SidePanelService::Get(profile);
+                if (!service) {
+                  LOG(WARNING) << "browseros: SidePanelService not found";
+                  return;
+                }
+
+                auto result = service->BrowserosToggleSidePanelForTab(
+                    *extension, profile, tab_id,
+                    /*include_incognito_information=*/true,
+                    /*desired_state=*/std::nullopt);
+
+                if (!result.has_value()) {
+                  LOG(WARNING) << "browseros: Agent toggle failed: "
+                               << result.error();
+                } else {
+                  LOG(INFO) << "browseros: Agent toggle result: "
+                            << result.value();
+                }
+              },
+              bwi))
+          .SetActionId(kActionBrowserOSAgent)
+          .SetText(u"Assistant")
+          .SetTooltipText(u"Ask BrowserOS")
+          .SetImage(ui::ImageModel::FromResourceId(IDR_PRODUCT_LOGO_16))
+          .SetProperty(actions::kActionItemPinnableKey,
+                       std::underlying_type_t<actions::ActionPinnableState>(
+                           actions::ActionPinnableState::kNotPinnable))
+          .Build());
+
   if (HistorySidePanelCoordinator::IsSupported()) {
     root_action_item_->AddChild(
         SidePanelAction(SidePanelEntryId::kHistory, IDS_HISTORY_TITLE,
