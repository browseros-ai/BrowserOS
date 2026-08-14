diff --git a/extensions/browser/extension_function_histogram_value.h b/extensions/browser/extension_function_histogram_value.h
index 804539041fa01756f00e3cc7a8b396e649291ffe..631bff3dfcf468a252c451007a3fedaac4a3b9ec 100644
--- a/extensions/browser/extension_function_histogram_value.h
+++ b/extensions/browser/extension_function_histogram_value.h
@@ -2018,6 +2018,7 @@ enum HistogramValue {
   BROWSER_OS_EXECUTEJAVASCRIPT = 1955,
   BROWSER_OS_CLICKCOORDINATES = 1956,
   BROWSER_OS_TYPEATCOORDINATES = 1957,
+  BROWSER_OS_TOGGLEAGENTSPLIT = 1958,
   // Last entry: Add new entries above, then run:
   // tools/metrics/histograms/update_extension_histograms.py
   ENUM_BOUNDARY
