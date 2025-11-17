diff --git a/chrome/install_static/chromium_install_modes.h b/chrome/install_static/chromium_install_modes.h
index 7de32cf745755..ae4e81676bf89 100644
--- a/chrome/install_static/chromium_install_modes.h
+++ b/chrome/install_static/chromium_install_modes.h
@@ -33,17 +33,17 @@ inline constexpr auto kInstallModes = std::to_array<InstallConstants>({
             L"",  // Empty install_suffix for the primary install mode.
         .logo_suffix = L"",  // No logo suffix for the primary install mode.
         .app_guid =
-            L"",  // Empty app_guid since no integration with Google Update.
-        .base_app_name = L"Chromium",              // A distinct base_app_name.
-        .base_app_id = L"Chromium",                // A distinct base_app_id.
-        .browser_prog_id_prefix = L"ChromiumHTM",  // Browser ProgID prefix.
+            L"{CF887152-7CB8-4393-84CC-1BACF0EDE1D1}",  // BlockBrowser app GUID.
+        .base_app_name = L"BlockBrowser",              // A distinct base_app_name.
+        .base_app_id = L"BlockBrowser",                // A distinct base_app_id.
+        .browser_prog_id_prefix = L"BOSHTML",  // Browser ProgID prefix.
         .browser_prog_id_description =
-            L"Chromium HTML Document",         // Browser ProgID description.
-        .pdf_prog_id_prefix = L"ChromiumPDF",  // PDF ProgID prefix.
+            L"BlockBrowser HTML Document",         // Browser ProgID description.
+        .pdf_prog_id_prefix = L"BOSPDF",  // PDF ProgID prefix.
         .pdf_prog_id_description =
-            L"Chromium PDF Document",  // PDF ProgID description.
+            L"BlockBrowser PDF Document",  // PDF ProgID description.
         .active_setup_guid =
-            L"{7D2B3E1D-D096-4594-9D8F-A6667F12E0AC}",  // Active Setup
+            L"{0EF5669B-7FD7-4138-A91F-E466631ADE97}",  // Active Setup
                                                         // GUID.
         .legacy_command_execute_clsid =
             L"{A2DF06F9-A21A-44A8-8A99-8B9C84F29160}",  // CommandExecuteImpl
