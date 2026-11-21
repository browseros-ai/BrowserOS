"""Exercise build and release key handling with fixture files and Docker stubs."""

from itertools import takewhile
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import textwrap
import unittest


DOCKER_DIR = Path(__file__).resolve().parent
WORKFLOW = DOCKER_DIR.parents[5] / '.github/workflows/neo-sdk-image.yml'


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

    def test_provider_keys_build_a_keyed_image_with_or_without_jev(self):
        for with_jev in (False, True):
            with self.subTest(with_jev=with_jev):
                flags = []
                secrets = []
                keys = ['capsolver', 'twocaptcha']
                if with_jev:
                    keys.append('jev')
                for name in keys:
                    key = self.root / f'{name}.key'
                    key.write_text('fixture-key')
                    flags += [f'--{name}-key-file', str(key)]
                    secrets.append(f'id={name}_api_key,src={key}')
                result = self.build(*flags)
                self.assertEqual(result.returncode, 97, result.stderr)
                args = self.arguments.read_text().splitlines()
                self.assertEqual(args[args.index('--target') + 1], 'neo-sdk-keyed')
                self.assertEqual([value for flag, value in zip(args, args[1:]) if flag == '--secret'], secrets)


@unittest.skipUnless(WORKFLOW.is_file() and shutil.which('jq'), 'release tests need the workflow and jq')
class ReleaseKeyTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory())).resolve()
        self.output = self.root / 'outputs'
        self.pushes = self.root / 'docker-commands'
        self.key_directory = self.root / 'neo-sdk-keys'
        docker = self.root / 'docker'
        docker.write_text('#!/bin/sh\nprintf "%s\\n" "$1" >> "$DOCKER_STUB_COMMANDS"\n')
        docker.chmod(0o700)
        self.environment = {
            'PATH': str(self.root) + os.pathsep + os.environ['PATH'],
            'DOCKER_STUB_COMMANDS': str(self.pushes),
            'RUNNER_TEMP': str(self.root),
            'GITHUB_OUTPUT': str(self.output),
            'CAPSOLVER_API_KEY': 'fixture-capsolver',
            'TWOCAPTCHA_API_KEY': 'fixture-twocaptcha',
            'JEV_API_KEY': '',
            'TAG': 'fixture-image',
            'VERSION': 'fixture-version',
            'ARCH': 'arm64',
            'IMAGE': 'fixture-registry/neo-sdk',
        }

    def run_step(self, name, **variables):
        step = WORKFLOW.read_text().split(f'      - name: {name}\n', 1)[1]
        script = step.split('        run: |\n', 1)[1]
        lines = takewhile(lambda line: not line or line.startswith('          '), script.splitlines())
        return subprocess.run(
            ['bash', '-c', textwrap.dedent('\n'.join(lines))],
            env={**self.environment, **variables}, capture_output=True, text=True, check=False,
        )

    def prepare(self, jev_key):
        result = self.run_step('Prepare the CAPTCHA keys', JEV_API_KEY=jev_key)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        captured = self.output.read_text()
        output = dict(line.split('=', 1) for line in captured.splitlines())
        for secret in ('fixture-capsolver', 'fixture-twocaptcha', 'fixture-jev'):
            self.assertNotIn(secret, result.stdout + result.stderr + captured)
        return result, output

    def push(self, outputs, provenance):
        self.pushes.unlink(missing_ok=True)
        directory = self.root / 'neo-sdk'
        directory.mkdir(exist_ok=True)
        (directory / 'provenance.json').write_text(json.dumps(provenance))
        return self.run_step('Push the accepted image', EXPECT_JEV_KEY=outputs['expect_jev_key'])

    def test_release_without_jev_warns_and_publishes_both_provider_keys(self):
        result, outputs = self.prepare('')
        self.assertIn('::warning::', result.stdout)
        self.assertEqual(outputs['expect_jev_key'], 'false')
        self.assertEqual(shlex.split(outputs['args']), [
            '--capsolver-key-file', str(self.key_directory / 'capsolver'),
            '--twocaptcha-key-file', str(self.key_directory / 'twocaptcha'),
        ])
        self.assertFalse((self.key_directory / 'jev').exists())
        for fields in ({}, {'jevKey': False}):
            with self.subTest(fields=fields):
                pushed = self.push(outputs, {'captchaKeys': ['capsolver', '2captcha'], **fields})
                self.assertEqual(pushed.returncode, 0, pushed.stdout + pushed.stderr)
                self.assertEqual(self.pushes.read_text().splitlines(), ['tag', 'push'])

    def test_release_with_jev_checks_that_the_image_has_the_key(self):
        result, outputs = self.prepare('fixture-jev')
        self.assertNotIn('::warning::', result.stdout)
        self.assertEqual(outputs['expect_jev_key'], 'true')
        key = self.key_directory / 'jev'
        self.assertEqual(shlex.split(outputs['args'])[-2:], ['--jev-key-file', str(key)])
        self.assertEqual(key.stat().st_mode & 0o777, 0o600)
        for fields in ({}, {'jevKey': False}, {'jevKey': True}):
            with self.subTest(fields=fields):
                result = self.push(outputs, {'captchaKeys': ['capsolver', '2captcha'], **fields})
                if fields.get('jevKey') is True:
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertEqual(self.pushes.read_text().splitlines(), ['tag', 'push'])
                else:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse(self.pushes.exists())

    def test_release_requires_both_providers(self):
        for missing in ('CAPSOLVER_API_KEY', 'TWOCAPTCHA_API_KEY'):
            with self.subTest(missing=missing):
                result = self.run_step('Prepare the CAPTCHA keys', **{missing: '', 'JEV_API_KEY': 'fixture-jev'})
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('::error::', result.stdout)
                self.assertFalse(self.key_directory.exists())
        _, outputs = self.prepare('')
        result = self.push(outputs, {'captchaKeys': ['capsolver'], 'jevKey': False})
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.pushes.exists())
