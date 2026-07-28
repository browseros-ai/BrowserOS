diff --git a/base/version_info/version_info.h b/base/version_info/version_info.h
index 1f54eef6f4b0a..ad12af4d982c2 100644
--- a/base/version_info/version_info.h
+++ b/base/version_info/version_info.h
@@ -30,6 +30,11 @@ constexpr std::string_view GetVersionNumber() {
   return PRODUCT_VERSION;
 }
 
+// Returns the BrowserOS version number, e.g. "0.30.0.0".
+constexpr std::string_view GetBrowserOSVersionNumber() {
+  return BROWSEROS_VERSION;
+}
+
 // Returns the major component (aka the milestone) of the version as an int,
 // e.g. 6 when the version is "6.0.490.1".
 int GetMajorVersionNumberAsInt();
