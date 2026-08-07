#!/usr/bin/env python3
"""Linux signing module for BrowserOS"""

from ...core.step import Step, step
from ...core.context import Context
from ...lib.utils import log_info


@step("sign_linux", phase="sign", platforms=("linux",))
class LinuxSignModule(Step):
    produces = []
    requires = []
    description = "Linux code signing (no-op)"

    def validate(self, ctx: Context) -> None:
        pass

    def execute(self, ctx: Context) -> None:
        log_info("Code signing is not required for Linux packages")


def check_signing_environment() -> bool:
    """Linux doesn't require signing environment"""
    return True
