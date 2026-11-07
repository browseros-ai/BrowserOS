"""Product registry — one package per product.

Adding a product: create products/<id>/product.py with a
ProductDescriptor.define() call (plus its ServerBundle definitions),
import it below, and run `browseros product doctor` until green. The
original BrowserOS product (id "browseros") moved to browseros-human;
asking for it fails as an unknown product.
"""

from typing import Dict

from ..core.products import ProductDescriptor
from .browserclaw.product import BROWSERCLAW_PRODUCT, BROWSERCLAW_SERVER_BUNDLE

DEFAULT_PRODUCT_ID = BROWSERCLAW_PRODUCT.id

PRODUCTS: Dict[str, ProductDescriptor] = {}
for _product in (BROWSERCLAW_PRODUCT,):
    if _product.id in PRODUCTS:
        raise ValueError(f"Duplicate product id: {_product.id}")
    PRODUCTS[_product.id] = _product

SERVER_BUNDLES = (BROWSERCLAW_SERVER_BUNDLE,)

__all__ = [
    "DEFAULT_PRODUCT_ID",
    "PRODUCTS",
    "SERVER_BUNDLES",
    "BROWSERCLAW_PRODUCT",
    "BROWSERCLAW_SERVER_BUNDLE",
]
