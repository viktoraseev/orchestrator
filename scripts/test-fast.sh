#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib --bins --all-features
cargo test --doc --all-features

test_targets=()
while IFS= read -r test_target; do
    test_targets+=(--test "$test_target")
done < <(sed -n '/^\[\[test\]\]$/,/^$/s/^name = "\(.*\)"$/\1/p' Cargo.toml)

if [[ ${#test_targets[@]} -eq 0 ]]; then
    echo "error: Cargo.toml не содержит Cucumber test targets" >&2
    exit 1
fi

cargo test --all-features "${test_targets[@]}" -- --tags 'not @slow'
