#!/usr/bin/env python3
"""Exercise host keeps against the image's real browser, agent, and idle janitor."""

import json
import sys
import time

from accept_client import call_run, connect, host


def check(url: str, socket_path: str, token_file: str) -> dict:
    client = connect(url)

    def request(method, path, body=""):
        reply = host(socket_path, token_file, path, method, body)
        assert reply.get("status") == 200, f"{method} {path}: {reply}"
        return reply["body"]

    def run(code):
        reply = call_run(client, code, {"hostData": {"check": "tab-keep"}})
        assert reply["meta"].get("status") == "ok", reply
        return (reply.get("value") or {}).get("value")

    def tabs():
        return {tab["tabId"]: tab for tab in request("GET", "/v1/tabs")["items"]}

    def keep(tab, body=""):
        return request("PUT", f"/v1/tabs/{tab}/keep", body)

    def until(predicate, seconds, message):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(1)
        raise AssertionError(message)

    original = request("GET", "/v1/settings")["settings"]
    created = []
    try:
        request("PATCH", "/v1/settings", '{"tabs":{"idleCloseSeconds":60}}')
        opened = run("""
            const ids = {};
            for (const name of ['held', 'deleted', 'expired', 'control']) {
                const tab = await context.newPage();
                await tab.setContent('<input aria-label="Value">');
                ids[name] = tab.tabId;
            }
            return ids;
        """)
        created.extend(opened.values())
        held, deleted, expired, control = (opened[name] for name in opened)
        for tab in (held, deleted, expired):
            assert keep(tab) == {"tabId": tab, "keep": {}}
        listed = tabs()
        assert all(listed[tab].get("keep") == {} for tab in (held, deleted, expired)), listed

        # Observe a sweep instead of assuming its phase or duration. Host inventory reads
        # do not touch the agent's idle clocks.
        until(lambda: control not in tabs(), 180, "idle control survived the cleanup deadline")
        listed = tabs()
        assert all(listed.get(tab, {}).get("keep") == {} for tab in (held, deleted, expired)), listed

        acted = run(f"""
            const tab = context.pages().find(p => p.tabId === {held});
            await tab.getByLabel('Value').fill('kept');
            const value = await tab.getByLabel('Value').inputValue();
            await tab.close();
            return value;
        """)
        assert acted == "kept", acted
        until(lambda: held not in tabs(), 30, "page.close did not close the kept tab")
        assert request("DELETE", f"/v1/tabs/{deleted}/keep") == {"tabId": deleted}
        released_at = time.monotonic()
        limited = keep(expired, '{"ttlSeconds":1}')
        assert isinstance(limited["keep"].get("expiresAt"), int), limited
        until(lambda: "keep" not in tabs()[expired], 30, "TTL did not release the keep")
        listed = tabs()
        assert "keep" not in listed[deleted] and "keep" not in listed[expired], listed
        # The control proved these tabs were already idle. Neither may close at the next
        # sweep until a fresh full idle interval has passed.
        while time.monotonic() - released_at < 55:
            listed = tabs()
            assert deleted in listed and expired in listed, f"release lost the full idle interval: {listed}"
            time.sleep(2)
        until(lambda: deleted not in tabs() and expired not in tabs(), 180,
              "released or expired tab survived a full idle interval and the next sweep")

        owned = run("""
            const task = await neo.createTask('tab-keep acceptance');
            await page.setContent('<title>kept task</title>');
            return { task: task.id, tab: page.tabId };
        """)
        created.append(owned["tab"])
        keep(owned["tab"], "{}")
        run(f"const task = await neo.task({json.dumps(owned['task'])}); await task.close();")
        until(lambda: owned["tab"] not in tabs(), 30, "task.close did not close the kept tab")

        missing = host(socket_path, token_file, "/v1/tabs/9223372036854775807/keep", "PUT", "{}")
        assert missing.get("status") == 404 and missing["body"]["code"] == "tab_not_found", missing
        assert request("DELETE", "/v1/tabs/9223372036854775807/keep") == {"tabId": 9223372036854775807}
        tools = [tool["name"] for tool in client.tools()]
        assert not any("keep" in name.lower() for name in tools), tools
        return {"ok": True, "detail": "keeps outlived the idle control; agent fill, read, page.close and task.close passed; DELETE and TTL restarted the full idle interval"}
    finally:
        request("PUT", "/v1/settings", json.dumps(original))
        if created:
            run(f"for (const tab of context.pages()) if ({json.dumps(created)}.includes(tab.tabId)) await tab.close();")


if __name__ == "__main__":
    try:
        print(json.dumps(check(*sys.argv[1:])))
    except Exception as error:
        print(json.dumps({"ok": False, "detail": str(error)}))
        sys.exit(1)
