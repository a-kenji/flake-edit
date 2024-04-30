alias d := doc
alias l := lint
alias f := format
alias uf := update-flake-dependencies
alias uc := update-cargo-dependencies
alias r := run
alias t := cargo-test
alias b := build
alias rr := run-release
alias cw := cargo-watch

default:
    @just --choose

clippy:
    cargo clippy --all-targets --all-features

actionlint:
    nix develop .#full --command actionlint

deny:
    cargo deny check

cargo-test:
    cargo test

cargo-diet:
    nix develop .#full --command cargo diet

cargo-mutants:
    nix develop .#full --command cargo mutants

cargo-tarpaulin:
    nix develop .#full --command cargo tarpaulin --out html --exclude-files "benches/*"

cargo-diff:
    nix develop .#full --command cargo public-api diff

format:
    nix fmt

lint:
    -nix develop .#full --command cargo diet
    -nix develop .#full --command cargo deny check licenses
    -nix develop .#full --command typos
    nix develop .#full --command lychee *.md
    nix develop .#full --command treefmt --fail-on-change
    -nix develop .#full --command cargo udeps
    -nix develop .#full --command cargo machete
    -nix develop .#full --command cargo outdated
    nix develop .#mdShell --command mdsh --frozen
    nix develop .#actionlintShell --command actionlint --ignore SC2002
    cargo check --future-incompat-report
    nix flake check

run:
    cargo run

build:
    cargo build

run-release:
    cargo run --release

doc:
    cargo doc --open --offline

# Update and then commit the `Cargo.lock` file
update-cargo-dependencies:
    cargo update
    git add Cargo.lock
    git commit Cargo.lock -m "update(cargo): \`Cargo.lock\`"

# Future incompatibility report, run regularly
cargo-future:
    cargo check --future-incompat-report

update-flake-dependencies:
    nix flake update --commit-lock-file

cargo-watch:
    cargo watch -x check -x test -x build

# build all examples
examples:
    nix develop --command $SHELL
    example_list=$(cargo build --example 2>&1 | sed '1,2d' | awk '{print $1}')

    # Build each example
    # shellcheck disable=SC2068
    for example in ${example_list[@]}; do
    cargo build --example "$example"
    done

examples-msrv:
    set -x
    nix develop .#msrvShell --command
    rustc --version
    cargo --version
    example_list=$(cargo build --example 2>&1 | grep -v ":")

    # Build each example
    # shellcheck disable=SC2068
    for example in ${example_list[@]}; do
    cargo build --example "$example"
    done
