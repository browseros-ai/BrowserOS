## trios-launch Skill

### Launch trios_app
1. Open terminal
2. Run: `cd /Users/playra/BrowserOS-full/trios && ./trios_app`
3. Click black triangle icon in status bar

### Or via .app bundle
1. `open ~/Applications/trios.app`
2. Click status bar icon

### If panel does not open
1. Check Accessibility: System Settings > Privacy > Accessibility > trios_app [U+2705]
2. Try Cmd+Shift+T global hotkey
3. Check logs: `cat /tmp/trios_debug.log`

### Health Check
`curl -s http://127.0.0.1:9105/health`

### Troubleshooting
- killall trios; rm -rf ~/Applications/trios.app
- Rebuild: swiftc -O -o trios_app ...
- Restart macOS if permissions stuck