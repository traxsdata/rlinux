#!/usr/bin/env bash
# Fail if any Rust source in the tree is not rustfmt-formatted.
. "$(dirname "$0")/lib.sh"
step "rustfmtcheck"
"${MAKE[@]}" rustfmtcheck
echo "ci: rustfmt OK"
