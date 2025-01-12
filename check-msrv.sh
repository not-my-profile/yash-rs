set -Ceu

if [ "$*" = "" ]; then quiet='--quiet'; else quiet=''; fi

set -x

# Make sure the files are clean before we modify them
git diff --exit-code -- Cargo.lock Cargo.toml
trap 'git checkout -- Cargo.lock Cargo.toml' EXIT

update_workspace_member() {
    cat >| Cargo.toml <<EOF
[workspace]
members = ["$1"]
resolver = "2"
EOF
}

update_workspace_member yash-arith
cargo +nightly update -Z direct-minimal-versions
cargo +1.58.0 test --package yash-arith -- $quiet

update_workspace_member yash-builtin
cargo +nightly update -Z direct-minimal-versions
cargo +1.67.0 test --package yash-builtin -- $quiet

update_workspace_member yash-env
cargo +nightly update -Z direct-minimal-versions
cargo +1.67.0 test --package yash-env -- $quiet

update_workspace_member yash-fnmatch
cargo +nightly update -Z direct-minimal-versions
cargo +1.60.0 test --package yash-fnmatch -- $quiet

update_workspace_member yash-quote
cargo +nightly update -Z direct-minimal-versions
cargo +1.56.0 test --package yash-quote -- $quiet

update_workspace_member yash-semantics
cargo +nightly update -Z direct-minimal-versions
cargo +1.67.0 test --package yash-semantics -- $quiet

update_workspace_member yash-syntax
cargo +nightly update -Z direct-minimal-versions
cargo +1.67.0 test --package yash-syntax -- $quiet
cargo +1.67.0 test --package yash-syntax --features annotate-snippets -- $quiet

update_workspace_member yash
cargo +nightly update -Z direct-minimal-versions
cargo +1.67.0 test --package yash -- $quiet
