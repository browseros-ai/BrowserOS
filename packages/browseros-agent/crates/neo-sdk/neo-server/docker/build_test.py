"""Exercise the image builder's key flags with fixture files and a Docker stub."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


DOCKER_DIR = Path(__file__).resolve().parent


@unittest.skipUnless(shutil.which('jq'), 'build.sh checks for jq before calling Docker')
class BuildKeyTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        recipe = self.root / 'packages/browseros-agent/crates/neo-sdk/neo-server/docker'
        recipe.mkdir(parents=True)
        shutil.copyfile(DOCKER_DIR / 'browser.lock', recipe / 'browser.lock')
        self.arguments = self.root / 'docker-arguments'
        docker = self.root / 'docker'
        docker.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$DOCKER_STUB_ARGUMENTS"\nexit 97\n')
        docker.chmod(0o700)
        self.environment = {
            'PATH': str(self.root) + os.pathsep + os.environ['PATH'],
            'DOCKER_STUB_ARGUMENTS': str(self.arguments),
            'NEO_SDK_SOURCE_ROOT': str(self.root.resolve()),
            'NEO_SDK_SOURCE_REVISION': 'a' * 40,
            'NEO_SDK_SOURCE_DIRTY': 'false',
        }

    def build(self, *flags):
        return subprocess.run(
            ['bash', str(DOCKER_DIR / 'build.sh'), '--platform', 'linux/arm64', *flags],
            env=self.environment, capture_output=True, text=True, check=False,
        )

    def test_a_jev_key_selects_the_keyed_target_and_invalidates_its_cache(self):
        key = self.root / 'fixture jev key'
        key.write_text('fixture-key')
        result = self.build('--jev-key-file', str(key))
        self.assertEqual(result.returncode, 97, result.stderr)
        args = self.arguments.read_text().splitlines()
        self.assertEqual(args[args.index('--target') + 1], 'neo-sdk-keyed')
        self.assertEqual(args[args.index('--secret') + 1], f'id=jev_api_key,src={key}')
        self.assertEqual(args[args.index('--no-cache-filter') + 1], 'neo-sdk-keyed')
        self.assertNotIn('fixture-key', result.stdout + result.stderr)

    def test_a_missing_or_empty_jev_key_stops_before_docker(self):
        key = self.root / 'fixture-jev-key'
        for empty in (False, True):
            with self.subTest(empty=empty):
                if empty:
                    key.touch()
                result = self.build('--jev-key-file', str(key))
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn('missing or empty', result.stderr)
                self.assertFalse(self.arguments.exists())

    def test_a_build_without_key_flags_uses_the_keyless_target(self):
        result = self.build()
        self.assertEqual(result.returncode, 97, result.stderr)
        args = self.arguments.read_text().splitlines()
        self.assertEqual(args[args.index('--target') + 1], 'neo-sdk')
        self.assertNotIn('--secret', args)
        self.assertNotIn('--no-cache-filter', args)
