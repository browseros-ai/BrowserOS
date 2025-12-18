diff --git a/chrome/install_static/chromium_install_modes.h b/chrome/install_static/chromium_install_modes.h
index 7de32cf745755..18c95e7046ef1 100644
--- a/chrome/install_static/chromium_install_modes.h
+++ b/chrome/install_static/chromium_install_modes.h
@@ -33,47 +33,47 @@ inline constexpr auto kInstallModes = std::to_array<InstallConstants>({
             L"",  // Empty install_suffix for the primary install mode.
         .logo_suffix = L"",  // No logo suffix for the primary install mode.
         .app_guid =
-            L"",  // Empty app_guid since no integration with Google Update.
-        .base_app_name = L"Chromium",              // A distinct base_app_name.
-        .base_app_id = L"Chromium",                // A distinct base_app_id.
-        .browser_prog_id_prefix = L"ChromiumHTM",  // Browser ProgID prefix.
+            L"{CF887152-7CB8-4393-84CC-1BACF0EDE1D1}",  // BrowserOS app GUID.
+        .base_app_name = L"BrowserOS",              // A distinct base_app_name.
+        .base_app_id = L"BrowserOS",                // A distinct base_app_id.
+        .browser_prog_id_prefix = L"BOSHTML",  // Browser ProgID prefix.
         .browser_prog_id_description =
-            L"Chromium HTML Document",         // Browser ProgID description.
-        .pdf_prog_id_prefix = L"ChromiumPDF",  // PDF ProgID prefix.
+            L"BrowserOS HTML Document",         // Browser ProgID description.
+        .pdf_prog_id_prefix = L"BOSPDF",  // PDF ProgID prefix.
         .pdf_prog_id_description =
-            L"Chromium PDF Document",  // PDF ProgID description.
+            L"BrowserOS PDF Document",  // PDF ProgID description.
         .active_setup_guid =
-            L"{7D2B3E1D-D096-4594-9D8F-A6667F12E0AC}",  // Active Setup
+            L"{0EF5669B-7FD7-4138-A91F-E466631ADE97}",  // Active Setup
                                                         // GUID.
         .legacy_command_execute_clsid =
-            L"{A2DF06F9-A21A-44A8-8A99-8B9C84F29160}",  // CommandExecuteImpl
+            L"{AFDDB293-0724-49E5-A4EC-1096BF6C84AF}",  // CommandExecuteImpl
                                                         // CLSID.
-        .toast_activator_clsid = {0x635EFA6F,
-                                  0x08D6,
-                                  0x4EC9,
-                                  {0xBD, 0x14, 0x8A, 0x0F, 0xDE, 0x97, 0x51,
-                                   0x59}},  // Toast Activator CLSID.
-        .elevator_clsid = {0xD133B120,
-                           0x6DB4,
-                           0x4D6B,
-                           {0x8B, 0xFE, 0x83, 0xBF, 0x8C, 0xA1, 0xB1,
-                            0xB0}},  // Elevator CLSID.
-        .elevator_iid = {0xb88c45b9,
-                         0x8825,
-                         0x4629,
-                         {0xb8, 0x3e, 0x77, 0xcc, 0x67, 0xd9, 0xce,
-                          0xed}},  // IElevator IID and TypeLib
-        // {B88C45B9-8825-4629-B83E-77CC67D9CEED}.
-        .tracing_service_clsid = {0x83f69367,
-                                  0x442d,
-                                  0x447f,
-                                  {0x8b, 0xcc, 0x0e, 0x3f, 0x97, 0xbe, 0x9c,
-                                   0xf2}},  // SystemTraceSession CLSID.
-        .tracing_service_iid = {0xa3fd580a,
-                                0xffd4,
-                                0x4075,
-                                {0x91, 0x74, 0x75, 0xd0, 0xb1, 0x99, 0xd3,
-                                 0xcb}},  // ISystemTraceSessionChromium IID and
+        .toast_activator_clsid = {0xE76CCE76,
+                                  0x27A7,
+                                  0x46D3,
+                                  {0x9E, 0xED, 0xCC, 0x8C, 0x5E, 0xD7, 0xBE,
+                                   0x72}},  // Toast Activator CLSID.
+        .elevator_clsid = {0x29ED629C,
+                           0x1F0E,
+                           0x47D1,
+                           {0xA6, 0x84, 0x93, 0x97, 0xAC, 0xDB, 0x71,
+                            0xAB}},  // Elevator CLSID.
+        .elevator_iid = {0x2F95E08F,
+                         0x118A,
+                         0x49B7,
+                         {0xA0, 0xE0, 0x49, 0x1C, 0x36, 0x80, 0x82,
+                          0x76}},  // IElevator IID and TypeLib
+        // {2F95E08F-118A-49B7-A0E0-491C36808276}.
+        .tracing_service_clsid = {0xC39C8575,
+                                  0x9F42,
+                                  0x4599,
+                                  {0x96, 0xF1, 0x19, 0xDB, 0x7A, 0xEB, 0x51,
+                                   0xAF}},  // SystemTraceSession CLSID.
+        .tracing_service_iid = {0x16F5E1C4,
+                                0xD385,
+                                0x4D38,
+                                {0xB0, 0xA1, 0x6E, 0x47, 0x6B, 0x39, 0xC0,
+                                 0x35}},  // ISystemTraceSessionChromium IID and
                                           // TypeLib
         .default_channel_name =
             L"",  // Empty default channel name since no update integration.
