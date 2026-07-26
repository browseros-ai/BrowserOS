"""
Allow running build package as module: python -m bos_build
"""
from .browseros import app

if __name__ == "__main__":
    app()
