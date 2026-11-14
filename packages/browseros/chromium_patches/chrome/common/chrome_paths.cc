diff --git a/chrome/common/chrome_paths.cc b/chrome/common/chrome_paths.cc
index 32d01c008e1341eb8142cd80e1c91afe7928ed42..1bea0de54fb3768be7efc83c03d91c1da1895b19 100644
--- a/chrome/common/chrome_paths.cc
+++ b/chrome/common/chrome_paths.cc
@@ -507,6 +507,34 @@ bool PathProvider(int key, base::FilePath* result) {
       create_dir = true;
       break;
 
+    case chrome::DIR_BROWSEROS_BUNDLED_EXTENSIONS:
+#if BUILDFLAG(IS_MAC)
+      cur = base::apple::FrameworkBundlePath();
+      cur = cur.Append(FILE_PATH_LITERAL("Resources"))
+                .Append(FILE_PATH_LITERAL("browseros_extensions"));
+#else
+      if (!base::PathService::Get(base::DIR_MODULE, &cur)) {
+        return false;
+      }
+      cur = cur.Append(FILE_PATH_LITERAL("browseros_extensions"));
+#endif
+      break;
+
+#if BUILDFLAG(IS_WIN)
+    // BrowserOS: Windows has no per-profile "External Extensions" path of its
+    // own. Resolve the same "<user-data-dir>/External Extensions" directory
+    // macOS and Chromium Linux use, so every --user-data-dir grid can pick up
+    // dropped-in manifests on startup. Deliberately does not create the
+    // directory; the writer is responsible for it.
+    case chrome::DIR_USER_EXTERNAL_EXTENSIONS: {
+      if (!base::PathService::Get(chrome::DIR_USER_DATA, &cur)) {
+        return false;
+      }
+      cur = cur.Append(FILE_PATH_LITERAL("External Extensions"));
+      break;
+    }
+#endif  // BUILDFLAG(IS_WIN)
+
     default:
       return false;
   }
