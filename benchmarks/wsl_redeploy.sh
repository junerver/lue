#!/usr/bin/env bash
# Build the Rust workspace on the WSL side and install the native binary.
# (The Python build is gone; this is a pure cargo pipeline now.)
set -e
rm -rf ~/lue-src
# rsync (not cp): DrvFS copies of the target dir are multi-GB and hang the deploy
rsync -a --exclude .git --exclude target \
    --exclude .zcode --exclude .idea \
    /mnt/e/GitHub/lue/ ~/lue-src/

source "$HOME/.cargo/env"
cd ~/lue-src
cargo build --release --workspace 2>&1 | tail -1

mkdir -p ~/.local/bin
# cp+mv: replacing a running binary fails with ETXTBSY when the user is reading
cp target/release/lue ~/.local/bin/lue-rs.new
mv -f ~/.local/bin/lue-rs.new ~/.local/bin/lue-rs
# `lue` is an alias of the native binary
ln -sfn ~/.local/bin/lue-rs ~/.local/bin/lue
~/.local/bin/lue-rs --version

rm -rf ~/lue-src
ls -la ~/.local/bin/lue-rs
