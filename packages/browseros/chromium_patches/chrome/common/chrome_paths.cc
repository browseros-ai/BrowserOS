diff --git a/chrome/common/chrome_paths.cc b/chrome/common/chrome_paths.cc
index 32d01c008e1341eb8142cd80e1c91afe7928ed42..58b85f0cee91f235c28fe7f4b510b4e1562c1e9b 100644
--- a/chrome/common/chrome_paths.cc
+++ b/chrome/common/chrome_paths.cc
@@ -507,6 +507,19 @@ bool PathProvider(int key, base::FilePath* result) {
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
     default:
       return false;
   }
