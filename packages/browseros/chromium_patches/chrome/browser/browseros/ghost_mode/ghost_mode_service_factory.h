diff --git a/chrome/browser/browseros/ghost_mode/ghost_mode_service_factory.h b/chrome/browser/browseros/ghost_mode/ghost_mode_service_factory.h
new file mode 100644
index 0000000000000..a7b8c9d0e1f2a
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/ghost_mode_service_factory.h
@@ -0,0 +1,56 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_SERVICE_FACTORY_H_
+#define CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_SERVICE_FACTORY_H_
+
+#include "base/no_destructor.h"
+#include "chrome/browser/profiles/profile_keyed_service_factory.h"
+
+class Profile;
+
+namespace browseros::ghost_mode {
+
+class GhostModeService;
+
+// Factory for creating GhostModeService instances per profile.
+// Ghost Mode is a per-profile service since different profiles may have
+// different browsing patterns and privacy settings.
+class GhostModeServiceFactory : public ProfileKeyedServiceFactory {
+ public:
+  // Returns the GhostModeService for the given profile.
+  // Creates the service if it doesn't exist yet.
+  static GhostModeService* GetForProfile(Profile* profile);
+  
+  // Returns the GhostModeService for the given profile if it exists.
+  // Does not create the service.
+  static GhostModeService* GetForProfileIfExists(Profile* profile);
+  
+  // Returns the singleton factory instance.
+  static GhostModeServiceFactory* GetInstance();
+  
+  GhostModeServiceFactory(const GhostModeServiceFactory&) = delete;
+  GhostModeServiceFactory& operator=(const GhostModeServiceFactory&) = delete;
+
+ private:
+  friend base::NoDestructor<GhostModeServiceFactory>;
+  
+  GhostModeServiceFactory();
+  ~GhostModeServiceFactory() override;
+  
+  // BrowserContextKeyedServiceFactory:
+  std::unique_ptr<KeyedService> BuildServiceInstanceForBrowserContext(
+      content::BrowserContext* context) const override;
+  
+  // Ghost Mode is not available in incognito/off-the-record profiles
+  bool ServiceIsCreatedWithBrowserContext() const override;
+  
+  // Don't create service for incognito profiles
+  bool ServiceIsNULLWhileTesting() const override;
+  
+  // Context behavior for incognito
+  content::BrowserContext* GetBrowserContextToUse(
+      content::BrowserContext* context) const override;
+};
+
+}  // namespace browseros::ghost_mode
+
+#endif  // CHROME_BROWSER_BROWSEROS_GHOST_MODE_GHOST_MODE_SERVICE_FACTORY_H_
