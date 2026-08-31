#!/usr/bin/env bash
# Force-reinstall lue_rs into the lue-reader pipx venv (fixes stale .so).
set -e
VENV="$HOME/.local/share/pipx/venvs/lue-reader"
PY="$VENV/bin/python"

"$PY" -m pip uninstall -y lue-rs 2>&1 | tail -1 || true

rm -rf ~/lue-src
cp -r /mnt/e/GitHub/lue ~/lue-src
rm -rf ~/lue-src/.git ~/lue-src/build ~/lue-src/lue_reader.egg-info ~/lue-src/images ~/lue-src/rust/target

source "$HOME/.cargo/env"
cd ~/lue-src/rust
~/.local/bin/maturin build --release 2>&1 | tail -1

"$PY" -m pip install --force-reinstall --no-deps \
    ~/lue-src/rust/target/wheels/lue_rs-0.1.0-cp312-cp312-manylinux_2_34_x86_64.whl 2>&1 | tail -1
rm -rf ~/lue-src

echo "--- installed .so ---"
ls -la "$VENV/lib/python3.12/site-packages/lue_rs/"*.so