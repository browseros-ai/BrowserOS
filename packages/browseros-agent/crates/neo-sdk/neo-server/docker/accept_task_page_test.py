"""task-page's other-tab shape sets tabs.scope to browser for itself: the check must fail, never
pass, when it cannot read the scope first, cannot set it, or cannot put it back."""

import unittest
from unittest.mock import patch

import accept_task_page as check


class Server:
    """A stand-in for the runs and the settings API: `tabs.scope` starts at `start` (None when
    it cannot be read), and the PATCH to browser or back can be made to fail."""

    def __init__(self, start, set_ok=True, restore_ok=True):
        self.scope, self.set_ok, self.restore_ok = start, set_ok, restore_ok
        self.patches: list[str] = []

    def tab_scope(self, scope=None):
        if scope is None:
            return self.scope
        self.patches.append(scope)
        applied = self.set_ok if scope == "browser" else self.restore_ok
        if applied:
            self.scope = scope
        return self.scope

    def goto(self, task):
        return 1

    def close(self, task):
        return None


def other_tab(server, i, problems):
    """The shape's repetitions all pass whenever the scope is browser, as on a sound server."""
    return server.scope == "browser"


class OtherTabScopeTest(unittest.TestCase):
    def accept(self, server):
        with patch.dict(check.SHAPES, {"other-tab": other_tab}):
            return check.accept(server, 3, ["other-tab"])

    def test_passes_and_puts_the_scope_back(self):
        server = Server("task")
        result = self.accept(server)
        self.assertTrue(result["ok"], result)
        self.assertEqual(server.patches, ["browser", "task"])
        self.assertEqual(server.scope, "task")

    def test_a_scope_it_cannot_read_is_never_changed_and_fails(self):
        server = Server(None)
        result = self.accept(server)
        self.assertFalse(result["ok"], result)
        self.assertEqual(server.patches, [], "no override without the value to put back")
        self.assertIn("could not be read", result["problems"][0])

    def test_a_scope_it_cannot_set_fails(self):
        server = Server("task", set_ok=False)
        result = self.accept(server)
        self.assertFalse(result["ok"], result)
        self.assertEqual(server.scope, "task")

    def test_a_scope_it_cannot_put_back_fails_though_every_repetition_passed(self):
        server = Server("task", restore_ok=False)
        result = self.accept(server)
        self.assertFalse(result["ok"], result)
        self.assertEqual(result["wrong"], {"other-tab": 3})
        self.assertIn("not put back", result["problems"][0])


if __name__ == "__main__":
    unittest.main()
