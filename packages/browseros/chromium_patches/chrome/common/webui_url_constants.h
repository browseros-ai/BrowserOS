diff --git a/chrome/common/webui_url_constants.h b/chrome/common/webui_url_constants.h
index d2e895dbc4a16..a6882132cdf40 100644
--- a/chrome/common/webui_url_constants.h
+++ b/chrome/common/webui_url_constants.h
@@ -32,6 +32,7 @@ namespace chrome {
 // needed.
 // Please keep in alphabetical order, with OS/feature specific sections below.
 inline constexpr char kChromeUIAboutHost[] = "about";
+inline constexpr char kBlockBrowserFirstRun[] = "browseros-first-run";
 inline constexpr char kChromeUIAboutURL[] = "chrome://about/";
 inline constexpr char kChromeUIAccessCodeCastHost[] = "access-code-cast";
 inline constexpr char kChromeUIAccessCodeCastURL[] =
@@ -56,6 +57,8 @@ inline constexpr char kChromeUIBatchUploadURL[] = "chrome://batch-upload/";
 inline constexpr char kChromeUIBluetoothInternalsHost[] = "bluetooth-internals";
 inline constexpr char kChromeUIBookmarksHost[] = "bookmarks";
 inline constexpr char kChromeUIBookmarksURL[] = "chrome://bookmarks/";
+inline constexpr char kChromeUIClashOfGptsHost[] = "clash-of-gpts";
+inline constexpr char kChromeUIClashOfGptsURL[] = "chrome://clash-of-gpts/";
 inline constexpr char kChromeUIBrowsingTopicsInternalsHost[] =
     "topics-internals";
 inline constexpr char kChromeUICertificateViewerHost[] = "view-cert";
