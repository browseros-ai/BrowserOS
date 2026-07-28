diff --git a/chrome/browser/importer/importer_uma.cc b/chrome/browser/importer/importer_uma.cc
index d50675a79eb60..fa28adace33b7 100644
--- a/chrome/browser/importer/importer_uma.cc
+++ b/chrome/browser/importer/importer_uma.cc
@@ -25,6 +25,7 @@ enum ImporterTypeMetrics {
 #if BUILDFLAG(IS_WIN)
   IMPORTER_METRICS_EDGE = 7,
 #endif
+  IMPORTER_METRICS_CHROME = 8,
 
   // Insert new values here. Never remove any existing values, as this enum is
   // used to bucket a UMA histogram, and removing values breaks that.
@@ -59,6 +60,9 @@ void LogImporterUseToMetrics(const std::string& metric_postfix,
     case user_data_importer::TYPE_BOOKMARKS_FILE:
       metrics_type = IMPORTER_METRICS_BOOKMARKS_FILE;
       break;
+    case user_data_importer::TYPE_CHROME:
+      metrics_type = IMPORTER_METRICS_CHROME;
+      break;
   }
 
   // Note: This leaks memory, which is the expected behavior as the factory
