# 🚀 TRIOS Launch Guide

## ✅ Correct Application

| Property | Value |
|----------|-------|
| **Name** | trios.app |
| **Location** | `~/Applications/trios.app` |
| **Binary** | `~/Applications/trios.app/Contents/MacOS/trios` |
| **Project** | `/Users/playra/BrowserOS-full/trios/` |

## 🎯 How to Launch

### Method 1: Double-click (Recommended)
1. Open **Finder** → **Applications** (or press `Cmd+Shift+A`)
2. Find **trios** app (black "T" icon)
3. **Double-click** to launch

### Method 2: From Dock
- If trios is in Dock, **click the icon**

### Method 3: From Terminal
```bash
open ~/Applications/trios.app
```

### Method 4: Direct Binary
```bash
~/Applications/trios.app/Contents/MacOS/trios
```

## 🖱️ How to Open Panel

1. **Look at the top-right corner** of your screen (status bar)
2. Find **TRIOS AGENT** icon (next to Wi-Fi / battery)
3. **Click it** → panel slides in from the right

**Keyboard shortcut:** `Cmd+Shift+T` (global hotkey)

## 📁 Project Files

| File | Purpose |
|------|---------|
| `~/Applications/trios.app` | **The app** (launch this!) |
| `./trios_app` | Raw binary (for developers) |
| `./build.sh` | Build script |
| `./main.swift` | Entry point |
| `./rings/SR-02/ChatViewModel.swift` | Chat logic |
| `./BR-OUTPUT/` | UI components |

## 🔧 Rebuild After Changes

```bash
cd /Users/playra/BrowserOS-full/trios
./build.sh
```

Then copy to Applications:
```bash
cp ./trios_app ~/Applications/trios.app/Contents/MacOS/trios
```

## ⚠️ What NOT to Open

- ❌ `/Applications/BrowserOS.app` — **Different app!** (old standalone)
- ❌ `./trios_app` directly — works but no Dock icon
- ❌ Any old cached copies in `~/Applications/trios_app_backup.app`

## 🎮 Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Cmd+Shift+T` | Toggle panel |
| `Cmd+1` | Chat tab (Local + BrowserOS) |
| `Cmd+2` | Git tab (GitHub + GitButler) |
| `Cmd+3` | Terminal tab |
| `Cmd+4` | Queen tab |
| `Cmd+5` | Settings tab |

## 🩺 Troubleshooting

**"Panel doesn't open"**
→ Kill and relaunch: `pkill -9 trios && open ~/Applications/trios.app`

**"App won't launch"**
→ Check Console for crash logs

**"Status bar icon missing"**
→ Check if another instance is running: `pgrep -x trios`

---

**Single source of truth:** `~/Applications/trios.app`
