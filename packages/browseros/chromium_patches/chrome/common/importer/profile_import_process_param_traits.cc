diff --git a/chrome/common/importer/profile_import_process_param_traits.cc b/chrome/common/importer/profile_import_process_param_traits.cc
index 87bc6ce8a9db1..69d89745af967 100644
--- a/chrome/common/importer/profile_import_process_param_traits.cc
+++ b/chrome/common/importer/profile_import_process_param_traits.cc
@@ -45,4 +45,32 @@ bool StructTraits<chrome::mojom::ImportedPasswordFormDataView,
   return true;
 }
 
+// static
+bool StructTraits<chrome::mojom::ImportedCookieEntryDataView,
+                  browseros_importer::ImportedCookieEntry>::
+    Read(chrome::mojom::ImportedCookieEntryDataView data,
+         browseros_importer::ImportedCookieEntry* out) {
+  if (!data.ReadHostKey(&out->host_key) || !data.ReadName(&out->name) ||
+      !data.ReadValue(&out->value) || !data.ReadPath(&out->path) ||
+      !data.ReadSameSite(&out->same_site) ||
+      !data.ReadPriority(&out->priority) ||
+      !data.ReadSourceScheme(&out->source_scheme)) {
+    return false;
+  }
+
+  out->expires_utc = base::Time::FromDeltaSinceWindowsEpoch(
+      base::Microseconds(data.expires_utc()));
+  out->creation_utc = base::Time::FromDeltaSinceWindowsEpoch(
+      base::Microseconds(data.creation_utc()));
+  out->last_access_utc = base::Time::FromDeltaSinceWindowsEpoch(
+      base::Microseconds(data.last_access_utc()));
+  out->last_update_utc = base::Time::FromDeltaSinceWindowsEpoch(
+      base::Microseconds(data.last_update_utc()));
+  out->is_secure = data.is_secure();
+  out->is_httponly = data.is_httponly();
+  out->source_port = data.source_port();
+  out->is_persistent = data.is_persistent();
+  return true;
+}
+
 }  // namespace mojo
