# PFTerminal

PFTerminal is a crypto-native AI services terminal based on the open-source Codex CLI. It defaults to Ambient's GLM 5.2 model and is intended to become one secure interface for crypto-native AI workflows.

## Current Focus

- Ambient API-key onboarding by default
- Ambient GLM 5.2 as the default model
- Codex-level coding workflows in a local terminal
- Future crypto-native services such as authentication, Hyperliquid, GPU rentals, staking, borrowing, and related workflows

## Running Locally

From this repository:

```shell
cd codex-rs
cargo build -p codex-cli
```

Launch it from the workspace you want PFTerminal to inspect:

```shell
cd /home/postfiat/repos
/home/postfiat/repos/PfTerminal/codex-rs/target/debug/codex
```

The binary name is still `codex` for upstream compatibility. The npm package also exposes a `pfterminal` alias for product-facing installs.

## Upstream

PFTerminal is based on the open-source Codex CLI project. Keep upstream changes isolated through the `upstream` remote and land PFTerminal changes through this repository.

This repository is licensed under the [Apache-2.0 License](LICENSE).
