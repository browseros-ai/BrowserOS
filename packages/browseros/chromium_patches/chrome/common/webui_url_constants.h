diff --git a/chrome/common/webui_url_constants.h b/chrome/common/webui_url_constants.h
index 47c543c844b3348c0360fb2a38c214aeab41944f..e29e526938cdfb2d280dc7c23beb0e7ab715729d 100644
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
