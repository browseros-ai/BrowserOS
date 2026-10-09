diff --git a/ui/webui/resources/tools/generate_grd.py b/ui/webui/resources/tools/generate_grd.py
index 7ff7e7e99304817095fbea3d3a3d29ce71e2492c..9652872d48345d41440929f86a58a096dfe961ae 100644
--- a/ui/webui/resources/tools/generate_grd.py
+++ b/ui/webui/resources/tools/generate_grd.py
@@ -37,6 +37,10 @@
 #   input_files_base_dir:
 #     The base directory for the paths in |input_files|. |input_files| and
 #     |input_files_base_dir| must either both be provided or both be omitted.
+#
+#   ignore_missing_input_files:
+#     If set, files from |input_files| that do not exist under
+#     |input_files_base_dir| are skipped.
 
 import argparse
 import json
@@ -122,6 +126,7 @@ def main(argv):
   parser.add_argument('--root-gen-dir', required=True)
   parser.add_argument('--input-files', nargs="*")
   parser.add_argument('--input-files-base-dir')
+  parser.add_argument('--ignore-missing-input-files', action='store_true')
   parser.add_argument('--output-files-base-dir', default='grit')
   parser.add_argument('--grdp-files', nargs="*")
   parser.add_argument('--resource-path-rewrites', nargs="*")
@@ -176,6 +181,14 @@ def main(argv):
           + 'input_files_base_dir'
         )
 
+        if args.ignore_missing_input_files:
+          real_base_dir = os.path.join(_CWD, '..', '..',
+                                       args.input_files_base_dir)
+          if args.input_files_base_dir.startswith(args.root_gen_dir + '/'):
+            real_base_dir = os.path.join(_CWD, args.input_files_base_dir)
+          if not os.path.exists(os.path.join(real_base_dir, filename)):
+            continue
+
         filepath = os.path.join(base_dir, filename).replace('\\', '/')
         grd_file.write(
           _generate_include_row(
