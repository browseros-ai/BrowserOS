diff --git a/chrome/common/webui_url_constants.h b/chrome/common/webui_url_constants.h
index 497865b22520bbe16bd6cc68b73ba90e25f70065..ec36673180644ff3cc74864a435164a13df6b357 100644
--- a/chrome/common/webui_url_constants.h
+++ b/chrome/common/webui_url_constants.h
@@ -34,6 +34,10 @@ namespace chrome {
 // needed.
 // Please keep in alphabetical order, with OS/feature specific sections below.
 inline constexpr char kChromeUIAboutHost[] = "about";
+inline constexpr char kChromeUIBrowserOSOnboardingHost[] =
+    "browseros-onboarding";
+inline constexpr char kChromeUIBrowserOSOnboardingURL[] =
+    "chrome://browseros-onboarding/";
 inline constexpr char kChromeUIAboutURL[] = "chrome://about/";
 inline constexpr char kChromeUIAccessCodeCastHost[] = "access-code-cast";
 inline constexpr char kChromeUIAccessCodeCastURL[] =
