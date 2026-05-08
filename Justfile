# Justfile
# https://github.com/casey/just
export REPO_URL := `grep '^repository' Cargo.toml | head -1 | cut -d'"' -f2`
REPO := "qa-server"
IMAGE := "skorotkiewicz/qa-server"

[private]
default:
    @just --list

build:
    cargo build --release

run *args:
    cargo run -- {{ args }}

fmt:
    cargo fmt
    cargo clippy --all-targets --all-features -- -D warnings
    # cargo shear --fix # first install shear: cargo install shear

check:
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

install-hook:
    #!/usr/bin/env bash
    cat > .git/hooks/pre-commit << 'EOF'
    #!/bin/sh
    set -e
    echo "Running pre-commit quality checks..."
    just check
    EOF
    chmod +x .git/hooks/pre-commit
    echo "Pre-commit hook installation confirmed."

remove-hook:
    rm .git/hooks/pre-commit
    echo "Pre-commit hook uninstallation confirmed."

add-tag:
    #!/usr/bin/env bash
    git push
    VERSION=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
    git tag "v${VERSION}"
    git commit -m "v${VERSION}"
    git push origin "v${VERSION}"
    echo "Created and pushed tag v${VERSION}"

remove-tag VERSION:
    git tag --delete {{ VERSION }}
    git push --delete origin {{ VERSION }}
    echo "Removed tag {{ VERSION }}"

build-image:
    docker build -t {{ REPO }} .
    docker tag "{{ REPO }}:latest" "{{ IMAGE }}:latest"
    echo "Build image {{ IMAGE }}:latest"

push-image: build-image
    docker push {{ IMAGE }}:latest
    echo "Pushed image {{ IMAGE }}:latest"

pull-image:
    docker pull {{ IMAGE }}:latest
    echo "Pulled image {{ IMAGE }}:latest"

# Run unit tests
test: fmt
    cargo test
