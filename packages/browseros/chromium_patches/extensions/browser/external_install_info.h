diff --git a/extensions/browser/external_install_info.h b/extensions/browser/external_install_info.h
index 44d216fb33fa837ec311f9c5594f7a286ef5026e..cef6df11da78279b546268bd7fd841012eab8509 100644
--- a/extensions/browser/external_install_info.h
+++ b/extensions/browser/external_install_info.h
@@ -14,6 +14,16 @@
 
 namespace extensions {
 
+// Scheduling urgency for this external-file install only. It is not persisted
+// in extension preferences or creation flags, so later updates retain their
+// normal priority unless their provider explicitly requests foreground work.
+enum class ExternalInstallPriority {
+  // Preserve Chromium's existing priority derived from install provenance.
+  kDefault,
+  // Installation is needed by a visible surface, without blocking the UI thread.
+  kForeground,
+};
+
 // Holds information about an external extension install from an external
 // provider.
 struct ExternalInstallInfo {
@@ -37,7 +47,9 @@ struct ExternalInstallInfoFile : public ExternalInstallInfo {
                           mojom::ManifestLocation crx_location,
                           int creation_flags,
                           bool mark_acknowledged,
-                          bool install_immediately);
+                          bool install_immediately,
+                          ExternalInstallPriority install_priority =
+                              ExternalInstallPriority::kDefault);
   ExternalInstallInfoFile(ExternalInstallInfoFile&& other);
   ~ExternalInstallInfoFile() override;
 
@@ -45,6 +57,7 @@ struct ExternalInstallInfoFile : public ExternalInstallInfo {
   base::FilePath path;
   mojom::ManifestLocation crx_location;
   bool install_immediately;
+  ExternalInstallPriority install_priority;
 };
 
 struct ExternalInstallInfoUpdateUrl : public ExternalInstallInfo {
