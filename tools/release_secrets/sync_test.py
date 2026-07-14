import unittest

from tools.release_secrets.sync import (
    ALLOWLIST,
    RELEASE_WORKFLOW_FILES,
    DotenvParseError,
    build_check_result,
    build_plan,
    parse_dotenv_text,
    scan_secret_refs_from_text,
    verify_dotenv_round_trip,
)


class DotenvParserTest(unittest.TestCase):
    def test_multiline_quoted_values_round_trip(self):
        parsed = parse_dotenv_text(
            'SPARKLE_PRIVATE_KEY="-----BEGIN PRIVATE KEY-----\n'
            "line-one\n"
            "line-two\n"
            '-----END PRIVATE KEY-----"\n'
            "BROWSEROS_AGENT_V2_KEY='-----BEGIN KEY-----\n"
            "agent-line\n"
            "-----END KEY-----'\n"
        )

        self.assertEqual(
            "-----BEGIN PRIVATE KEY-----\nline-one\nline-two\n-----END PRIVATE KEY-----",
            parsed["SPARKLE_PRIVATE_KEY"],
        )
        self.assertEqual(
            "-----BEGIN KEY-----\nagent-line\n-----END KEY-----",
            parsed["BROWSEROS_AGENT_V2_KEY"],
        )
        verify_dotenv_round_trip(parsed)

    def test_quotes_escapes_and_inline_comments(self):
        parsed = parse_dotenv_text(
            'DOUBLE="line\\nquote \\" ok \\\\ done"\n'
            "SINGLE='raw\\nvalue'\n"
            "SINGLE_ESCAPED='can\\'t \\\\ stop'\n"
            "UNQUOTED=value # ignored comment\n"
            "HASH=abc#def\n"
            "export EXPORTED = spaced\n"
        )

        self.assertEqual('line\nquote " ok \\ done', parsed["DOUBLE"])
        self.assertEqual("raw\\nvalue", parsed["SINGLE"])
        self.assertEqual("can't \\ stop", parsed["SINGLE_ESCAPED"])
        self.assertEqual("value", parsed["UNQUOTED"])
        self.assertEqual("abc#def", parsed["HASH"])
        self.assertEqual("spaced", parsed["EXPORTED"])

    def test_crlf_input_normalizes_multiline_values(self):
        parsed = parse_dotenv_text('A="one\r\ntwo"\r\nB=three\r\n')

        self.assertEqual("one\ntwo", parsed["A"])
        self.assertEqual("three", parsed["B"])
        verify_dotenv_round_trip(parsed)

    def test_unterminated_quote_raises_without_value_echo(self):
        with self.assertRaises(DotenvParseError) as ctx:
            parse_dotenv_text('SECRET="do-not-echo\n')

        self.assertIn("line 1", str(ctx.exception))
        self.assertNotIn("do-not-echo", str(ctx.exception))


class WorkflowSecretScannerTest(unittest.TestCase):
    def test_scans_dotted_and_bracket_secret_references(self):
        refs = scan_secret_refs_from_text(
            "env:\n"
            "  A: ${{ secrets.FOO }}\n"
            '  B: ${{ secrets["BAR_BAZ"] }}\n'
            "  C: ${{ secrets['QUX'] }}\n"
            "  D: ${{ vars.NOT_A_SECRET }}\n"
        )

        self.assertEqual({"BAR_BAZ", "FOO", "QUX"}, refs)

    def test_extension_feed_workflow_uses_only_r2_allowlisted_secrets(self):
        workflow = "release-extension-feeds.yml"
        self.assertIn(workflow, {path.name for path in RELEASE_WORKFLOW_FILES})

        consumers = {spec.name for spec in ALLOWLIST if workflow in spec.consumers}
        self.assertEqual(
            {
                "R2_ACCOUNT_ID",
                "R2_ACCESS_KEY_ID",
                "R2_SECRET_ACCESS_KEY",
                "R2_BUCKET",
            },
            consumers,
        )


class SecretPlanTest(unittest.TestCase):
    def test_slack_webhook_is_not_in_release_workflow_allowlist(self):
        plan = build_plan({"SLACK_WEBHOOK_URL": "unused"}, set())

        self.assertNotIn("SLACK_WEBHOOK_URL", {item.name for item in plan})

    def test_esigner_credential_id_is_optional_in_check(self):
        result = build_check_result(
            referenced={"ESIGNER_CREDENTIAL_ID", "ESIGNER_USERNAME"},
            existing={"ESIGNER_USERNAME"},
        )

        self.assertEqual(["ESIGNER_USERNAME"], result.present)
        self.assertEqual(["ESIGNER_CREDENTIAL_ID"], result.optional)
        self.assertEqual([], result.missing_required)

    def test_optional_agent_runner_secret_syncs_when_env_provides_it(self):
        plan = build_plan({"AGENT_RUNNER_JWT_SECRET": "jwt-secret"}, set())

        self.assertEqual(
            ("AGENT_RUNNER_JWT_SECRET", "set"),
            next(
                (item.name, item.status)
                for item in plan
                if item.name == "AGENT_RUNNER_JWT_SECRET"
            ),
        )


if __name__ == "__main__":
    unittest.main()
