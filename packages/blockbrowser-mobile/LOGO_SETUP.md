# BlockBrowser Logo Setup Guide

## Logo Source
Download the BlockBrowser logo from: https://pbs.twimg.com/profile_images/1688210251110039553/4AICYzQw_400x400.jpg

## Required Assets

### Mobile App (`packages/blockbrowser-mobile/assets/`)

1. **icon.png** (1024x1024)
   - App icon for iOS and Android
   - Download the logo and resize to 1024x1024
   - Save as: `assets/icon.png`

2. **adaptive-icon.png** (1024x1024)
   - Android adaptive icon foreground
   - Same as icon.png
   - Save as: `assets/adaptive-icon.png`

3. **splash.png** (2048x2048 or larger)
   - Splash screen image
   - Center the logo on colored background (#667eea)
   - Save as: `assets/splash.png`

4. **favicon.png** (48x48)
   - Web favicon
   - Resize logo to 48x48
   - Save as: `assets/favicon.png`

5. **favicon-placeholder.png** (32x32)
   - Placeholder for missing website favicons
   - Resize logo to 32x32
   - Save as: `assets/favicon-placeholder.png`

6. **logo-text.png** (optional, recommended)
   - Logo with "BlockBrowser" text
   - For use in homepage and about screens
   - Save as: `assets/logo-text.png`

### Desktop Browser (`packages/browseros/resources/`)

1. **icon.png** (256x256)
   - Application icon
   - Save as: `resources/icon.png`

2. **logo-128.png** (128x128)
   - For use in settings page
   - Save as: `resources/logo-128.png`

3. **logo-48.png** (48x48)
   - For use in extension icons
   - Save as: `resources/logo-48.png`

4. **logo-32.png** (32x32)
   - For use in tabs and small UI elements
   - Save as: `resources/logo-32.png`

## Quick Setup Script

```bash
# Download the logo
curl -o blockbrowser-logo-400.jpg "https://pbs.twimg.com/profile_images/1688210251110039553/4AICYzQw_400x400.jpg"

# Convert to PNG and resize (requires ImageMagick)
convert blockbrowser-logo-400.jpg -resize 1024x1024 icon-1024.png
convert blockbrowser-logo-400.jpg -resize 256x256 icon-256.png
convert blockbrowser-logo-400.jpg -resize 128x128 icon-128.png
convert blockbrowser-logo-400.jpg -resize 48x48 icon-48.png
convert blockbrowser-logo-400.jpg -resize 32x32 icon-32.png

# Create splash screen with background
convert -size 2048x2048 xc:#667eea \
  icon-1024.png -gravity center -composite \
  splash-2048.png

# Copy to appropriate locations
cp icon-1024.png packages/blockbrowser-mobile/assets/icon.png
cp icon-1024.png packages/blockbrowser-mobile/assets/adaptive-icon.png
cp splash-2048.png packages/blockbrowser-mobile/assets/splash.png
cp icon-48.png packages/blockbrowser-mobile/assets/favicon.png
cp icon-32.png packages/blockbrowser-mobile/assets/favicon-placeholder.png

cp icon-256.png packages/browseros/resources/icon.png
cp icon-128.png packages/browseros/resources/logo-128.png
cp icon-48.png packages/browseros/resources/logo-48.png
cp icon-32.png packages/browseros/resources/logo-32.png
```

## Manual Setup (Without ImageMagick)

1. Download the logo from the URL above
2. Use an image editor (Photoshop, GIMP, Figma, etc.) to:
   - Open the logo
   - Resize to each required dimension
   - Export as PNG with transparency
   - Save to the locations specified above

## Verification

After adding the logo files, verify:

```bash
# Mobile app assets
ls -lh packages/blockbrowser-mobile/assets/*.png

# Desktop browser assets
ls -lh packages/browseros/resources/*.png
```

All files should be present and have reasonable file sizes (1KB - 500KB depending on size).
