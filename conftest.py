"""Ensure the repo checkout (not an installed copy) is imported by the tests."""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
