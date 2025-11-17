#!/usr/bin/env python3
"""
Slack notification module for BlockBrowser build system
"""

import os
import json
import requests
from typing import Optional, List
import sys
import os

sys.path.append(os.path.dirname(os.path.dirname(__file__)))
from utils import log_info, log_warning, log_error, get_platform


def get_slack_webhook_url() -> Optional[str]:
    """Get Slack webhook URL from environment variable"""
    return os.environ.get("SLACK_WEBHOOK_URL")


def get_os_info() -> tuple[str, str]:
    """Get OS emoji and name for Slack notifications"""
    platform = get_platform()
    if platform == "macos":
        return "🍎", "macOS"
    elif platform == "windows":
        return "🪟", "Windows"
    elif platform == "linux":
        return "🐧", "Linux"
    else:
        return "💻", platform.capitalize()


def send_slack_notification(message: str, success: bool = True) -> bool:
    """Send a notification to Slack if webhook URL is configured"""
    webhook_url = get_slack_webhook_url()

    if not webhook_url:
        # Silently skip if no webhook configured
        return True

    # Choose emoji and color based on success status
    emoji = "✅" if success else "❌"
    color = "good" if success else "danger"

    # Get OS information
    os_emoji, os_name = get_os_info()

    # Create Slack message payload
    payload = {
        "attachments": [
            {
                "color": color,
                "fields": [
                    {
                        "title": "BlockBrowser Build",
                        "value": f"{emoji} {message}",
                        "short": False,
                    }
                ],
                "footer": f"{os_emoji} BlockBrowser Build System - {os_name}",
                "ts": None,  # Slack will use current timestamp
            }
        ]
    }

    try:
        response = requests.post(
            webhook_url,
            data=json.dumps(payload),
            headers={"Content-Type": "application/json"},
            timeout=10,
        )

        if response.status_code == 200:
            log_info(f"📲 Slack notification sent: {message}")
            return True
        else:
            log_warning(f"Slack notification failed with status {response.status_code}")
            return False

    except requests.RequestException as e:
        log_warning(f"Failed to send Slack notification: {e}")
        return False


def notify_build_started(build_type: str, arch: str) -> bool:
    """Notify that build has started"""
    _, os_name = get_os_info()
    message = f"Build started on {os_name} - {build_type} build for {arch}"
    return send_slack_notification(message, success=True)


def notify_build_step(step_name: str) -> bool:
    """Notify about a build step"""
    message = f"Running step: {step_name}"
    return send_slack_notification(message, success=True)


def notify_build_success(
    duration_mins: int, duration_secs: int, gcs_uris: Optional[List[str]] = None
) -> bool:
    """Notify that build completed successfully"""
    message = f"Build completed successfully in {duration_mins}m {duration_secs}s"

    # Add GCS URIs to message if provided
    if gcs_uris:
        message += f"\n\nUploaded artifacts ({len(gcs_uris)} files):"
        for uri in gcs_uris:
            # Convert gs:// URI to public URL for easier access
            if uri.startswith("gs://"):
                public_url = uri.replace("gs://", "https://storage.googleapis.com/")
                message += f"\n• {public_url}"
            else:
                message += f"\n• {uri}"

    return send_slack_notification(message, success=True)


def notify_build_failure(error_message: str) -> bool:
    """Notify that build failed"""
    message = f"Build failed: {error_message}"
    return send_slack_notification(message, success=False)


def notify_build_interrupted() -> bool:
    """Notify that build was interrupted"""
    message = "Build was interrupted by user"
    return send_slack_notification(message, success=False)


def notify_gcs_upload(architecture: str, gcs_uris: List[str]) -> bool:
    """Notify about GCS upload for a specific architecture"""
    if not gcs_uris:
        return True

    message = f"[{architecture}] Uploaded {len(gcs_uris)} artifact(s) to GCS"

    # Add URIs to message
    for uri in gcs_uris:
        # Convert gs:// URI to public URL
        if uri.startswith("gs://"):
            public_url = uri.replace("gs://", "https://storage.googleapis.com/")
            message += f"\n• {public_url}"
        else:
            message += f"\n• {uri}"

    return send_slack_notification(message, success=True)
