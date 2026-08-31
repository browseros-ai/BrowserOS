#!/usr/bin/env python3
"""Product-owned source resource declarations."""

from dataclasses import dataclass


@dataclass(frozen=True)
class SourceResources:
    """Product-owned inputs selected before shared resource staging."""

    product_id: str
    server_component: str
    extension_component: str
    extension_name: str
    onboarding_component: str
    external_extension_names: tuple[str, ...] = ("bugreporter",)


SOURCE_RESOURCES = {
    "browseros": SourceResources(
        product_id="browseros",
        server_component="server",
        extension_component="agent",
        extension_name="agent",
        onboarding_component="app-onboard",
    ),
    "browserclaw": SourceResources(
        product_id="browserclaw",
        server_component="claw-server-rust",
        extension_component="browserclaw",
        extension_name="browserclaw",
        onboarding_component="claw-onboard",
    ),
}


def source_resources_for_product(product_id: str) -> SourceResources:
    """Return source resource ownership for one product."""
    try:
        return SOURCE_RESOURCES[product_id]
    except KeyError as exc:
        valid = ", ".join(sorted(SOURCE_RESOURCES))
        raise ValueError(f"Unknown product '{product_id}'. Valid: {valid}") from exc
