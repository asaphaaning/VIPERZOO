# VIPERZOO

`VIPERZOO` is a homage to someone whose creative output around a classic online 
RPG made a deep impression on me when I was younger.

It is my own spin on his defining contribution: A research project on the 
same game, built on a reverse-engineered protocol and a deterministic engine.

## Workspace

### Crates

- `viperzoo-protocol` — Plaintext client and server protocol bodies.
- `viperzoo-capture` — Typed ingestion of research-tap records.
- `viperzoo-adapter-api` — Transport-neutral observations and actions.
- `viperzoo-engine` — Deterministic world-state reduction and runtime ownership.
- `viperzoo-adapter-frida` — Live client integration through Frida.
- `viperzoo-assets` — Client tile and object data.
- `viperzoo-navigation` — Deterministic path planning.
- `viperzoo-actions` — Reusable asynchronous action policies.
- `viperzoo-world` — Immutable world snapshots.
- `viperzoo-sdk` — The script-facing crate bundle.

### Applications

The repo contains dummy implementations demonstrating how to utilize the protocol as an author

- `viperzoo` — Attaches to and drives a live client.
- `viperzoo-live` — Follows a live observation stream.
- `viperzoo-replay` — Replays observations into a final world snapshot.
- `viperzoo-walk` — Walks to a coordinate on the current map.

## Commands

```powershell
cargo +nightly fmt --all --
cargo ci-clippy
cargo test --workspace

cargo run -p viperzoo
cargo run -p viperzoo-walk -- 10 1
```
