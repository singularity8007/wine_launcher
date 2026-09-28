#!/usr/bin/env bash
# Usage: source dev-env.sh
WL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export RUSTUP_HOME="$WL_ROOT/.toolchain/rustup"
export CARGO_HOME="$WL_ROOT/.toolchain/cargo"
export PATH="$CARGO_HOME/bin:$PATH"
