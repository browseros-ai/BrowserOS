diff --git a/chrome/common/chrome_paths.cc b/chrome/common/chrome_paths.cc
index eae95c5f35740..d350b9aff6705 100644
--- a/chrome/common/chrome_paths.cc
+++ b/chrome/common/chrome_paths.cc
@@ -518,6 +518,19 @@ bool PathProvider(int key, base::FilePath* result) {
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
