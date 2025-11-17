diff --git a/chrome/common/pref_names.h b/chrome/common/pref_names.h
index 0e898dc745b6e..c649ec745fbcd 100644
--- a/chrome/common/pref_names.h
+++ b/chrome/common/pref_names.h
@@ -1590,6 +1590,8 @@ inline constexpr char kImportDialogSavedPasswords[] =
     "import_dialog_saved_passwords";
 inline constexpr char kImportDialogSearchEngine[] =
     "import_dialog_search_engine";
+inline constexpr char kImportDialogExtensions[] =
+    "import_dialog_extensions";
 
 #if BUILDFLAG(IS_CHROMEOS)
 // Boolean controlling whether native client is force allowed by policy.
@@ -4271,6 +4273,29 @@ inline constexpr char kServiceWorkerToControlSrcdocIframeEnabled[] =
 // is set as a SharedWorker script URL.
 inline constexpr char kSharedWorkerBlobURLFixEnabled[] =
     "worker.shared_worker_blob_url_fix_enabled";
+
+// String containing the stable client ID for BlockBrowser metrics
+inline constexpr char kBlockBrowserMetricsClientId[] =
+    "browseros.metrics_client_id";
+
+// String containing the stable install ID for BlockBrowser metrics (Local State)
+inline constexpr char kBlockBrowserMetricsInstallId[] =
+    "browseros.metrics_install_id";
+
+// JSON string containing custom AI providers for BlockBrowser
+inline constexpr char kBlockBrowserCustomProviders[] = 
+    "browseros.custom_providers";
+
+// JSON string containing the list of AI providers and configuration
+inline constexpr char kBlockBrowserProviders[] = "browseros.providers";
+
+// String containing the default provider ID for BlockBrowser
+inline constexpr char kBlockBrowserDefaultProviderId[] = 
+    "browseros.default_provider_id";
+
+// Boolean that controls whether toolbar labels are shown for BlockBrowser actions
+inline constexpr char kBlockBrowserShowToolbarLabels[] =
+    "browseros.show_toolbar_labels";
 }  // namespace prefs
 
 #endif  // CHROME_COMMON_PREF_NAMES_H_
