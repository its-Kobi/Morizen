# Morizen

Morizen is a fork of [Horizon](https://github.com/horizon-rust/horizon): Minecraft: Java Edition 26.3
rewritten in Rust. The engine is Horizon's — a native client and a dedicated server that read the same
worlds, follow the same rules and generate the same terrain as the Java game, checked against it by
more than 300 differential harnesses. Morizen keeps that engine and gives it its own name and look.

> Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft. Morizen
> contains no Minecraft code, assets or data: you need your own legally obtained copy of Minecraft: Java
> Edition 26.3. Morizen is unofficial and is not made or endorsed by the Horizon project, Mojang or
> Microsoft.

## What Morizen changes

- **The launcher.** Its name, window title, text in every language, desktop entry and shortcuts say
  Morizen, and **Settings › About** has a **Fork** row that names the project it comes from.
- **The look.** The Blocks palette — the colours of the launcher and the project site — is warm neutrals
  with a gold accent instead of Horizon's cool violet. The game's own menus keep the neutral palette they
  had.
- **The accent picker** follows the new face: gold carries dark text, so the fitter lightens it instead of
  darkening it, and every pair still clears WCAG AA.

The game, the dedicated server and the mod loader are Horizon's work, unchanged.

## Project status

The engine is complete against vanilla: every subsystem Horizon scores (rendering, lighting, world
generation, entities and physics, redstone and block ticking, menus, audio, the network protocol) passes
its harnesses. That audit is Horizon's, and it lives in
[its README](https://github.com/horizon-rust/horizon#project-status); so is the engine's documentation.

Morizen itself is a **development build**, not a distribution:

- The launcher still fetches releases, the mod registry and its own updates from `horizon-rust/horizon`,
  so a Morizen launcher installs Horizon builds.
- Some internal package names and file paths still use `horizon` for compatibility.
- Some docs still describe Horizon, and the game's own screens still say "Horizon Settings".

## Build from source

Requirements: Rust 1.90 or newer, Python 3.10 or newer, JDK 25, and on Linux the ALSA and udev
development headers (`libasound2-dev libudev-dev` on Debian and Ubuntu). Development and the vanilla
harnesses run on Linux.

Morizen contains no game files, and `horizon-prepare` never downloads them. It builds its resources from
**your own copy** of Minecraft: Java Edition 26.3:

1. **Client:** install Java Edition with the official launcher and play 26.3 once. Morizen reads
   `versions/26.3/`, `libraries/` and `assets/` from the launcher's game directory; if yours lives
   elsewhere, pass `--minecraft-dir` or set `HORIZON_MINECRAFT_DIR`.
2. **Server jar:** the 26.3 server jar, from `downloads.server.url` in your launcher's
   `versions/26.3/26.3.json`. Pass it with `--server-jar` or set `HORIZON_SERVER_JAR`. It is only needed
   the first time.

```sh
cargo run --release -p horizon-prepare -- --prepare --server-jar ~/Downloads/server.jar
cargo run --release -p horizon-prepare -- sounds   # optional: sounds from your launcher install
cargo run --release -p horizon-client
```

Everything prepared lands in the gitignored `.local/`. It is derived from the game: never commit it or
share it.

The launcher is its own Cargo workspace:

```sh
cd launcher && cargo run --release
```

## Where to look

- [launcher/README.md](launcher/README.md) — everything the launcher does, with its command line.
- [AGENTS.md](AGENTS.md) — the map of the code, crate by crate.
- [docs/README.md](docs/README.md) — the developer documentation's index.
- [CONTRIBUTING.md](CONTRIBUTING.md) — how changes are made and checked.

## Licence and credits

[MIT](LICENSE), for the project's original work only. Dependencies keep their own licences;
[third-party notices](THIRD_PARTY_NOTICES.md).

Morizen is a fork of [Horizon](https://github.com/horizon-rust/horizon), made by
[Ludovic Marie](https://lu4ovic.com) ([@ludovic111](https://github.com/ludovic111)) and its
contributors. The engine, the harnesses and the documentation here are theirs.

Minecraft is a trademark of Mojang Synergies AB; Minecraft code and assets belong to Mojang and
Microsoft and are covered by the [Minecraft EULA](https://www.minecraft.net/en-us/eula). Decompiled game
source and prepared resources stay on your machine and are never published.
