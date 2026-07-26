diff --git a/chrome/browser/ui/webui/new_tab_footer/new_tab_footer_ui.cc b/chrome/browser/ui/webui/new_tab_footer/new_tab_footer_ui.cc
index 28d359d6a77bf..455ac9f707ef9 100644
--- a/chrome/browser/ui/webui/new_tab_footer/new_tab_footer_ui.cc
+++ b/chrome/browser/ui/webui/new_tab_footer/new_tab_footer_ui.cc
@@ -78,7 +78,7 @@ NewTabFooterUI::~NewTabFooterUI() = default;
 
 // static
 void NewTabFooterUI::RegisterProfilePrefs(PrefRegistrySimple* registry) {
-  registry->RegisterBooleanPref(prefs::kNtpFooterVisible, true);
+  registry->RegisterBooleanPref(prefs::kNtpFooterVisible, false);
 }
 
 void NewTabFooterUI::BindInterface(
