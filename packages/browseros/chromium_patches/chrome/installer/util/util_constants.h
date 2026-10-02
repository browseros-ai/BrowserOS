diff --git a/chrome/installer/util/util_constants.h b/chrome/installer/util/util_constants.h
index e9c61e09475afa893cdfc9047be01fbf597d61cf..69fddae7387a3c6dfaada018eaf22f5fccace50b 100644
--- a/chrome/installer/util/util_constants.h
+++ b/chrome/installer/util/util_constants.h
@@ -338,6 +338,9 @@ inline constexpr char kSelfDestruct[] = "self-destruct";
 // Show the embedded EULA dialog.
 inline constexpr char kShowEula[] = "show-eula";
 
+// Suppress the interactive install UI and first-install browser launch.
+inline constexpr char kSilent[] = "silent";
+
 // Saves the specified device management token to the registry.
 inline constexpr char kStoreDMToken[] = "store-dmtoken";
 
