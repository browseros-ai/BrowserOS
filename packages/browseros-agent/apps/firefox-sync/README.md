# firefox-sync

Copies saved passwords and signed-in sessions (cookies) from a Firefox profile into a running
BrowserOS neo. Run it again at any time to bring neo up to date with Firefox. It needs no
Chromium rebuild: it talks to the installed browser over CDP.

## Quick start

1. Start BrowserOS neo. Firefox may stay open.
2. From `packages/browseros-agent`, see what would change:

   ```sh
   bun run firefox-sync -- --dry-run
   ```

3. Sync:

   ```sh
   bun run firefox-sync
   ```

Run step 3 again whenever you want neo to catch up with Firefox.

## What a run does

| Data | Firefox source | Written to neo through | On a repeat run |
| --- | --- | --- | --- |
| Persistent cookies | `cookies.sqlite` (+ WAL) | `Storage.setCookies` on the browser target | Replaces cookies with the same name, domain and path |
| Session-only cookies | `sessionstore-backups/recovery.jsonlz4` or `sessionstore.jsonlz4` | Same | Same |
| Saved logins | `logins.json`, decrypted with `key4.db` | `chrome.passwordsPrivate` in a background `chrome://password-manager` tab | Adds new logins; updates only logins whose password changed in Firefox since the last run |

- Firefox is only read, from private copies of its files. The copies are deleted after
  reading.
- Nothing is deleted on either side. A password you delete in neo stays deleted until its
  password changes in Firefox.
- After writing, the cookie jar is read back. The report counts cookies neo actually
  holds, not calls that returned.
- Passwords, usernames and cookie values are never printed. The state file
  (`%LOCALAPPDATA%\browseros-firefox-sync\state.json` on Windows) holds login GUIDs and
  timestamps only.

## Options

| Option | Effect |
| --- | --- |
| `--profile <name\|dir\|path>` | Firefox profile. Default: the most recently used install default from `profiles.ini` |
| `--list-profiles` | List profiles and the one chosen by default |
| `--only cookies` / `--only passwords` | Sync one kind only |
| `--domain example.com` | Only that domain and its subdomains; repeat for more |
| `--missing-only` | Keep cookies neo already has (for example, a fresh login made in neo) |
| `--no-session-cookies` | Skip session-only cookies |
| `--dry-run` | Report what would change; write nothing |
| `--cdp-port <port>` | neo's CDP port. Default: `browseros.server.cdp_port` from neo's Local State, then 9110 |
| `--primary-password-stdin` | Read the Firefox Primary Password from stdin |

Exit status: 0 when everything selected reached neo, 1 when some items failed (listed in
the output), 2 on a fatal error.

## What is not carried over

- Cookies from Firefox containers, private windows and Total Cookie Protection partitions
  are counted and skipped. Chromium has one cookie jar per profile, and merging jars would
  mix accounts.
- HTTP authentication logins (`httpRealm`) and non-web logins are skipped. The password
  manager API only adds form logins.
- Session-only cookies live until neo closes, unless neo restores the previous session on
  startup.
- localStorage, IndexedDB and other site storage are not copied. A site that keeps its
  sign-in there asks you to sign in again.

## Run it on a schedule (optional)

Create a Windows scheduled task. neo must be running when the task fires:

```powershell
schtasks /Create /SC HOURLY /TN "BrowserOS firefox-sync" /TR "D:\Portable\bun\bin\bun.exe D:\home\github\BrowserOS\packages\browseros-agent\apps\firefox-sync\src\cli.ts"
```

Change the paths to match your Bun and repository locations.
