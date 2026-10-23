from pathlib import Path
import unittest

import yaml


class WorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        root = Path(__file__).resolve().parents[4]
        cls.jobs = yaml.safe_load((root / ".github/workflows/release-docker-captcha.yml").read_text())["jobs"]

    def tests_that_both_architectures_get_native_runtime_verification(self):
        verify = self.jobs["verify"]
        self.assertEqual(verify["runs-on"], "${{ matrix.runner }}")
        self.assertEqual(verify["strategy"]["matrix"]["include"], [
            {"arch": "amd64", "runner": "ubuntu-latest"},
            {"arch": "arm64", "runner": "ubuntu-24.04-arm"},
        ])
        build = next(s for s in verify["steps"] if s.get("with", {}).get("load"))
        self.assertEqual(build["with"]["platforms"], "linux/${{ matrix.arch }}")
        smoke = next(s for s in verify["steps"] if s.get("id") == "smoke")
        self.assertIn("for hosted in 1 0", smoke["run"])
        self.assertIn('release.py verify "http://127.0.0.1:$port" "$hosted"', smoke["run"])

    def tests_that_both_verification_legs_gate_publication(self):
        self.assertIn("bootstrap", self.jobs["verify"]["needs"])
        publish = self.jobs["publish"]
        self.assertIn("verify", publish["needs"])
        self.assertEqual(publish["if"], "inputs.push")
        paid = [s for s in self.jobs["verify"]["steps"] if "--paid" in s.get("run", "")]
        self.assertEqual(len(paid), 1)
        self.assertEqual(paid[0]["if"], "inputs.verify_paid && matrix.arch == 'amd64'")

    def tests_that_each_keyed_job_checks_privacy_and_cleans_its_own_secrets(self):
        for name in ("verify", "publish"):
            with self.subTest(job=name):
                job = self.jobs[name]
                self.assertEqual(job["environment"], "neo-captcha")
                self.assertNotIn("CAPSOLVER_API_KEY", job.get("env", {}))
                steps = job["steps"]
                prepare = next(i for i, s in enumerate(steps) if "umask 077" in s.get("run", ""))
                self.assertTrue(any("release.py private" in s.get("run", "") for s in steps[:prepare]))
                cleanup = steps[-1]
                self.assertEqual(cleanup["if"], "always()")
                self.assertIn('rm -f "$RUNNER_TEMP/neo-captcha-capsolver" "$RUNNER_TEMP/neo-captcha-2captcha"', cleanup["run"])
                for step in steps:
                    if step.get("uses", "").startswith("docker/build-push-action"):
                        self.assertFalse(step["with"]["provenance"])
                        self.assertFalse(step["with"]["sbom"])
                        self.assertNotIn("cache-to", step["with"])


if __name__ == "__main__":
    unittest.main()
