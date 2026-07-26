diff --git a/chrome/install_static/chromium_install_modes.h b/chrome/install_static/chromium_install_modes.h
index ee62888f89705a08a95f130505ebfafb246f4bc2..a0835e7f66dd12edeed11c672368f2d6e4de92a8 100644
--- a/chrome/install_static/chromium_install_modes.h
+++ b/chrome/install_static/chromium_install_modes.h
@@ -10,6 +10,7 @@
 #include <array>
 
 #include "chrome/app/chrome_dll_resource.h"
+#include "chrome/browser/browseros/buildflags.h"
 #include "chrome/common/chrome_icon_resources_win.h"
 #include "chrome/install_static/install_constants.h"
 
@@ -19,9 +20,84 @@ namespace install_static {
 // and user data directory paths. May be empty if no such dir is to be used.
 inline constexpr wchar_t kCompanyPathName[] = L"";
 
+struct ProductInstallIdentity {
+  const wchar_t* base_app_name;
+  const wchar_t* base_app_id;
+  const wchar_t* browser_prog_id_prefix;
+  const wchar_t* browser_prog_id_description;
+  const char* direct_launch_url_scheme;
+  const wchar_t* pdf_prog_id_prefix;
+  const wchar_t* pdf_prog_id_description;
+  const wchar_t* active_setup_guid;
+  const wchar_t* legacy_command_execute_clsid;
+  CLSID toast_activator_clsid;
+  CLSID elevator_clsid;
+  CLSID tracing_service_clsid;
+  const wchar_t* sandbox_sid_prefix;
+};
+
 // The brand-specific product name to be included as a component of the install
-// and user data directory paths.
-inline constexpr wchar_t kProductPathName[] = L"Chromium";
+// and user data directory paths. Per-product so BrowserOS and BrowserClaw get
+// disjoint user-data roots (and singletons) under %LOCALAPPDATA%.
+#if BUILDFLAG(BROWSEROS_PRODUCT_BROWSERCLAW)
+inline constexpr wchar_t kProductPathName[] = L"BrowserClaw";
+inline constexpr ProductInstallIdentity kProductInstallIdentity = {
+    .base_app_name = L"BrowserClaw",
+    .base_app_id = L"BrowserClaw",
+    .browser_prog_id_prefix = L"BClawHTML",
+    .browser_prog_id_description = L"BrowserClaw HTML Document",
+    .direct_launch_url_scheme = "browserclaw",
+    .pdf_prog_id_prefix = L"BClawPDF",
+    .pdf_prog_id_description = L"BrowserClaw PDF Document",
+    .active_setup_guid = L"{E9E65674-914E-4A29-83A9-A98D407446EC}",
+    .legacy_command_execute_clsid = L"",
+    .toast_activator_clsid = {0xD0A19C03,
+                              0xEE25,
+                              0x463B,
+                              {0xB3, 0x8F, 0x08, 0x51, 0x6D, 0x2B, 0x1A, 0x79}},
+    .elevator_clsid = {0x0AC4EA74,
+                       0xA61A,
+                       0x4807,
+                       {0xAF, 0xE5, 0x03, 0x70, 0x1D, 0x2B, 0x97, 0xDD}},
+    .tracing_service_clsid = {0x9F3CA910,
+                              0x142B,
+                              0x4C2C,
+                              {0xA6, 0x1E, 0xB2, 0x33, 0x5E, 0x2E, 0x67, 0xFD}},
+    .sandbox_sid_prefix =
+        L"S-1-15-2-3251537155-1984446955-2931258699-841473695-"
+        L"1938553385-"
+        L"924012147-",
+};
+#else
+inline constexpr wchar_t kProductPathName[] = L"BrowserOS";
+inline constexpr ProductInstallIdentity kProductInstallIdentity = {
+    .base_app_name = L"BrowserOS",
+    .base_app_id = L"BrowserOS",
+    .browser_prog_id_prefix = L"BOSHTML",
+    .browser_prog_id_description = L"BrowserOS HTML Document",
+    .direct_launch_url_scheme = "browseros",
+    .pdf_prog_id_prefix = L"BOSPDF",
+    .pdf_prog_id_description = L"BrowserOS PDF Document",
+    .active_setup_guid = L"{0EF5669B-7FD7-4138-A91F-E466631ADE97}",
+    .legacy_command_execute_clsid = L"{AFDDB293-0724-49E5-A4EC-1096BF6C84AF}",
+    .toast_activator_clsid = {0xE76CCE76,
+                              0x27A7,
+                              0x46D3,
+                              {0x9E, 0xED, 0xCC, 0x8C, 0x5E, 0xD7, 0xBE, 0x72}},
+    .elevator_clsid = {0x29ED629C,
+                       0x1F0E,
+                       0x47D1,
+                       {0xA6, 0x84, 0x93, 0x97, 0xAC, 0xDB, 0x71, 0xAB}},
+    .tracing_service_clsid = {0xC39C8575,
+                              0x9F42,
+                              0x4599,
+                              {0x96, 0xF1, 0x19, 0xDB, 0x7A, 0xEB, 0x51, 0xAF}},
+    .sandbox_sid_prefix =
+        L"S-1-15-2-3251537155-1984446955-2931258699-841473695-"
+        L"1938553385-"
+        L"924012148-",
+};
+#endif
 
 // The brand-specific safe browsing client name.
 inline constexpr char kSafeBrowsingName[] = "chromium";
@@ -43,50 +119,33 @@ inline constexpr auto kInstallModes = std::to_array<InstallConstants>({
         .install_suffix =
             L"",  // Empty install_suffix for the primary install mode.
         .logo_suffix = L"",  // No logo suffix for the primary install mode.
-        .app_guid =
-            L"",  // Empty app_guid since no integration with Google Update.
-        .base_app_name = L"Chromium",              // A distinct base_app_name.
-        .base_app_id = L"Chromium",                // A distinct base_app_id.
-        .browser_prog_id_prefix = L"ChromiumHTM",  // Browser ProgID prefix.
+        .app_guid = L"",
+        .base_app_name = kProductInstallIdentity.base_app_name,
+        .base_app_id = kProductInstallIdentity.base_app_id,
+        .browser_prog_id_prefix =
+            kProductInstallIdentity.browser_prog_id_prefix,
         .browser_prog_id_description =
-            L"Chromium HTML Document",  // Browser ProgID description.
-        .direct_launch_url_scheme = "chromium",
-        .pdf_prog_id_prefix = L"ChromiumPDF",  // PDF ProgID prefix.
+            kProductInstallIdentity.browser_prog_id_description,
+        .direct_launch_url_scheme =
+            kProductInstallIdentity.direct_launch_url_scheme,
+        .pdf_prog_id_prefix = kProductInstallIdentity.pdf_prog_id_prefix,
         .pdf_prog_id_description =
-            L"Chromium PDF Document",  // PDF ProgID description.
-        .active_setup_guid =
-            L"{7D2B3E1D-D096-4594-9D8F-A6667F12E0AC}",  // Active Setup
-                                                        // GUID.
+            kProductInstallIdentity.pdf_prog_id_description,
+        .active_setup_guid = kProductInstallIdentity.active_setup_guid,
         .legacy_command_execute_clsid =
-            L"{A2DF06F9-A21A-44A8-8A99-8B9C84F29160}",  // CommandExecuteImpl
-                                                        // CLSID.
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
-        .elevator_iid = {0xbb19a0e5,
-                         0xc6,
+            kProductInstallIdentity.legacy_command_execute_clsid,
+        .toast_activator_clsid = kProductInstallIdentity.toast_activator_clsid,
+        .elevator_clsid = kProductInstallIdentity.elevator_clsid,
+        .elevator_iid = {0xBB19A0E5,
+                         0x00C6,
                          0x4966,
-                         {0x94, 0xb2, 0x5a, 0xfe, 0xc6, 0xfe, 0xd9,
-                          0x3a}},  // IElevator IID and TypeLib
-        // {BB19A0E5-00C6-4966-94B2-5AFEC6FED93A}.
-        .tracing_service_clsid = {0x83f69367,
-                                  0x442d,
-                                  0x447f,
-                                  {0x8b, 0xcc, 0x0e, 0x3f, 0x97, 0xbe, 0x9c,
-                                   0xf2}},  // SystemTraceSession CLSID.
-        .tracing_service_iid = {0xa3fd580a,
-                                0xffd4,
+                         {0x94, 0xB2, 0x5A, 0xFE, 0xC6, 0xFE, 0xD9, 0x3A}},
+        .tracing_service_clsid = kProductInstallIdentity.tracing_service_clsid,
+        .tracing_service_iid = {0xA3FD580A,
+                                0xFFD4,
                                 0x4075,
-                                {0x91, 0x74, 0x75, 0xd0, 0xb1, 0x99, 0xd3,
-                                 0xcb}},  // ISystemTraceSessionChromium IID and
-                                          // TypeLib
+                                {0x91, 0x74, 0x75, 0xD0, 0xB1, 0x99, 0xD3,
+                                 0xCB}},
         .default_channel_name =
             L"",  // Empty default channel name since no update integration.
         .channel_strategy = ChannelStrategy::UNSUPPORTED,
@@ -100,10 +159,7 @@ inline constexpr auto kInstallModes = std::to_array<InstallConstants>({
             icon_resources::kHtmlDocIndex,  // HTML doc icon resource index.
         .pdf_doc_icon_resource_index =
             icon_resources::kPDFDocIndex,  // PDF doc icon resource index.
-        .sandbox_sid_prefix =
-            L"S-1-15-2-3251537155-1984446955-2931258699-841473695-"
-            L"1938553385-"
-            L"924012148-",  // App container sid prefix for sandbox.
+        .sandbox_sid_prefix = kProductInstallIdentity.sandbox_sid_prefix,
     },
 });
 
