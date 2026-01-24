diff --git a/chrome/test/data/webui/settings/import_data_dialog_test.ts b/chrome/test/data/webui/settings/import_data_dialog_test.ts
index cf54982f5a4b6..1cf13ad985867 100644
--- a/chrome/test/data/webui/settings/import_data_dialog_test.ts
+++ b/chrome/test/data/webui/settings/import_data_dialog_test.ts
@@ -47,6 +47,8 @@ suite('ImportDataDialog', function() {
   const browserProfiles: BrowserProfile[] = [
     {
       autofillFormData: true,
+      cookies: false,
+      extensions: false,
       favorites: true,
       history: true,
       index: 0,
@@ -57,6 +59,8 @@ suite('ImportDataDialog', function() {
     },
     {
       autofillFormData: true,
+      cookies: false,
+      extensions: false,
       favorites: true,
       history: false,  // Emulate unsupported import option
       index: 1,
@@ -67,6 +71,8 @@ suite('ImportDataDialog', function() {
     },
     {
       autofillFormData: false,
+      cookies: false,
+      extensions: false,
       favorites: true,
       history: false,
       index: 2,
