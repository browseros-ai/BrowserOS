diff --git a/chrome/browser/ui/toasts/toast_controller.h b/chrome/browser/ui/toasts/toast_controller.h
index c7fd61e831b161cb7a3869d352e08aaa581cdff9..7a9596f72350025a40322bbf0f90a5640de319b8 100644
--- a/chrome/browser/ui/toasts/toast_controller.h
+++ b/chrome/browser/ui/toasts/toast_controller.h
@@ -15,6 +15,7 @@
 #include "base/functional/callback_helpers.h"
 #include "base/memory/raw_ptr.h"
 #include "base/scoped_observation.h"
+#include "base/time/time.h"
 #include "base/timer/timer.h"
 #include "chrome/browser/ui/omnibox/omnibox_tab_helper.h"
 #include "content/public/browser/web_contents_observer.h"
@@ -65,6 +66,9 @@ struct ToastParams {
   std::vector<std::u16string> action_button_string_replacement_params;
   std::optional<int> body_string_cardinality_param;
   std::optional<std::u16string> body_string_override;
+  // Overrides the auto-dismiss timeout. When unset, the controller falls back
+  // to the default (or with-action) timeout.
+  std::optional<base::TimeDelta> timeout_override;
   std::optional<ui::ImageModel> image_override;
   std::unique_ptr<ui::MenuModel> menu_model;
   base::ScopedClosureRunner toast_close_callback;
