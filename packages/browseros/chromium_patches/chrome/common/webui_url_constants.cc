diff --git a/chrome/common/webui_url_constants.cc b/chrome/common/webui_url_constants.cc
index e5e724a22d015..4eaf639e58b84 100644
--- a/chrome/common/webui_url_constants.cc
+++ b/chrome/common/webui_url_constants.cc
@@ -72,6 +72,7 @@ bool IsSystemWebUIHost(std::string_view host) {
 // These hosts will also be suggested by BuiltinProvider.
 base::span<const base::cstring_view> ChromeURLHosts() {
   static constexpr auto kChromeURLHosts = std::to_array<base::cstring_view>({
+      kBlockBrowserFirstRun,
       kChromeUIAboutHost,
       kChromeUIAccessibilityHost,
 #if !BUILDFLAG(IS_ANDROID)
