diff --git a/extensions/browser/external_install_info.cc b/extensions/browser/external_install_info.cc
index 4605419ba144b5560d29a7c5dcc4f402e68b1c75..70b443ed262441d851036a0a219c2837b40d8893 100644
--- a/extensions/browser/external_install_info.cc
+++ b/extensions/browser/external_install_info.cc
@@ -22,12 +22,14 @@ ExternalInstallInfoFile::ExternalInstallInfoFile(
     mojom::ManifestLocation crx_location,
     int creation_flags,
     bool mark_acknowledged,
-    bool install_immediately)
+    bool install_immediately,
+    ExternalInstallPriority install_priority)
     : ExternalInstallInfo(extension_id, creation_flags, mark_acknowledged),
       version(version),
       path(path),
       crx_location(crx_location),
-      install_immediately(install_immediately) {}
+      install_immediately(install_immediately),
+      install_priority(install_priority) {}
 ExternalInstallInfoFile::ExternalInstallInfoFile(
     ExternalInstallInfoFile&& other) = default;
 
