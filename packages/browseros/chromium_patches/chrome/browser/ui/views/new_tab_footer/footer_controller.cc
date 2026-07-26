diff --git a/chrome/browser/ui/views/new_tab_footer/footer_controller.cc b/chrome/browser/ui/views/new_tab_footer/footer_controller.cc
index 6bd39ba34909e..0828b132a01e0 100644
--- a/chrome/browser/ui/views/new_tab_footer/footer_controller.cc
+++ b/chrome/browser/ui/views/new_tab_footer/footer_controller.cc
@@ -202,14 +202,7 @@ bool NewTabFooterController::ContentsViewFooterCotroller::
 
 bool NewTabFooterController::ContentsViewFooterCotroller::
     ShouldShowExtensionFooter(const GURL& url) {
-  if (ShouldSkipForErrorPage()) {
-    return false;
-  }
-
-  return ntp_footer::IsExtensionNtp(url, owner_->profile_) &&
-         owner_->profile_->GetPrefs()->GetBoolean(
-             prefs::kNTPFooterExtensionAttributionEnabled) &&
-         owner_->profile_->GetPrefs()->GetBoolean(prefs::kNtpFooterVisible);
+  return false;
 }
 
 void NewTabFooterController::UpdateFooterVisibilities(bool log_on_load_metric) {
