"""Build a complete local macOS app and register its acheflow:// link handler.

Usage: python3 native/macos/build-app.py --output target/acheflow-app
Or: python3 native/macos/build-app.py --output /Applications/Acheflow.app
Pass --no-register-links to build without changing LaunchServices discovery.
"""
from package import main


if __name__ == "__main__":
    main(app_only=True, register=True)
