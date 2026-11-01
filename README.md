<div align="center">

# Zero-Copy CoW Worlds

This Pumpkin Fork features a fast, in-memory Copy-on-Write (CoW) world management system built for minigames, arenas, and ephemeral server instances.

Inspired by AdvancedSlimePaper (ASWM), this Fork takes advantage of Rust's ownership model by wrapping chunk palettes in reference-counted pointers (Arc) and leveraging Arc::make_mut for lazy, subchunk-level cloning.

# Key Features
- **Zero-Copy Instance Spawning (< 0.1 ms)**: Spawning a new arena or game world from a loaded template never deep-clones block data. Instead, it copies only 8-byte Arc section pointers across chunks, allowing dozens of concurrent game instances to share the exact same underlying memory for untouched terrain.
- **Granular Subchunk Copy-on-Write (Arc::make_mut)**: Memory allocation only occurs at the exact 16×16×16 subchunk section where a block or biome is modified, leaving all other sections and instances completely untouched.
- **Sub-Millisecond Instant World Reset**: Resetting an arena (reset_instant) iterates strictly over the set of mutated chunks, swaps their Arc section pointers back to the immutable template, clears non-player entities, restores initial block entities, and broadcasts chunk updates to players — with zero disk I/O.
- **Disk I/O Bypass**: CoW world instances run purely in memory (CoWBackend), bypassing Anvil/region disk reads and writes completely.
- **First-Class WASM Plugin API & Built-in CLI**: WASM Host Interface (cow-world.wit): Plugins can programmatically manage templates and instances via pumpkin_plugin_api::cow_world::{load_template, spawn_instance, reset_instance, destroy_instance}.
- **In-Game / Console Command**: Built-in /cow <load|spawn|reset|destroy|stats> command for instant administration and testing.
- **Rebase-Friendly Architecture**: Most of the CoW engine lives in the isolated crates/pumpkin-cow-world workspace crate, keeping hooks in pumpkin-world and pumpkin-core minimal so upstream Pumpkin updates rebase cleanly.

</div>

<div align="center">

# Pumpkin

[![CI](https://github.com/Pumpkin-MC/Pumpkin/actions/workflows/rust.yml/badge.svg)](https://github.com/Pumpkin-MC/Pumpkin/actions/workflows/rust.yml)
[![Discord](https://img.shields.io/discord/1268592337445978193.svg?label=&logo=discord&logoColor=ffffff&color=7389D8&labelColor=6A7EC2)](https://discord.gg/wT8XjrjKkf)
[![License: GPL](https://img.shields.io/badge/License-GPLv3-yellow.svg)](https://opensource.org/licenses/gpl-3-0)

</div>

[Pumpkin](https://pumpkinmc.org/) is a Minecraft server built entirely in Rust, offering a fast, efficient,
and customizable experience. It prioritizes performance and player enjoyment while adhering to the core mechanics of the game.
<div align="center">

![Pumpkin Chunk Loading](./assets/pumpkin-chunk-loading.webp)

</div>

## Goals

- **Performance**: Leveraging multi-threading for maximum speed and efficiency.
- **Compatibility**: Supports the latest Java & Bedrock Minecraft server version while adhering to Vanilla game mechanics.
- **Security**: Prioritizes security by preventing known security exploits.
- **Flexibility**: Highly configurable, with the ability to disable unnecessary features.
- **Extensibility**: Provides a foundation for plugin development.

> [!IMPORTANT]
> Pumpkin is currently under heavy development.
>
> [See what needs to be done before the 1.0.0 Release](https://github.com/Pumpkin-MC/Pumpkin/issues/449)

## Features

- [x] Configuration (toml)
- [Tracking: Protocol](https://github.com/Pumpkin-MC/Pumpkin/issues/1401)
  - [x] Server Status/Ping
  - [x] Encryption
  - [x] Packet Compression
  - [x] Java Edition
  - [x] Bedrock Edition (W.I.P)
  - ...
- [Tracking: World](https://github.com/Pumpkin-MC/Pumpkin/issues/1403)
  - [x] Player Tab-list
  - [x] Scoreboard
  - [x] World Loading
  - [x] World Time
  - [x] World Borders
  - [x] World Saving
  - [x] Lighting
  - [x] Entity Spawning
  - [x] Bossbar
  - [x] Chunk Loading (Vanilla, Linear, Pump)
  - [Chunk Generation](https://github.com/Pumpkin-MC/Pumpkin/issues/36)
  - [x] Chunk Saving (Vanilla, Linear, Pump)
  - [Redstone](https://github.com/Pumpkin-MC/Pumpkin/issues/1402)
  - [x] Liquid Physics
  - ...
- [Tracking: Player](https://github.com/Pumpkin-MC/Pumpkin/issues/1405)
  - [x] Skins
  - [x] Teleport
  - [x] Movement
  - [x] Animation
  - [x] Inventory
  - [Combat](https://github.com/Pumpkin-MC/Pumpkin/issues/1404)
  - [x] Experience
  - [x] Hunger
  - [X] Off Hand
  - [X] Advancements (W.I.P)
  - [x] Eating
  - ...
- Entities
  - [x] Non-Living (Minecart, Eggs...) (W.I.P)
  - [x] Entity Effects
  - [x] Players
  - [x] Mobs (W.I.P)
  - [x] Animals (W.I.P)
  - [Entity AI](https://github.com/Pumpkin-MC/Pumpkin/issues/1406)
  - [x] Boss (W.I.P)
  - [x] Villagers (W.I.P)
  - [X] Entity Saving
- Server
  - [Plugins](https://github.com/Pumpkin-MC/Pumpkin/issues/1407)
  - [x] Query
  - [x] [Minecraft Server Management Protocol (MSMP)](https://minecraft.wiki/w/Minecraft_Server_Management_Protocol)
  - [x] Inventories
  - [x] Particles
  - [x] Chat
  - [Commands](https://github.com/Pumpkin-MC/Pumpkin/issues/15)
  - [x] Permissions
  - [x] Translations
- Proxy
  - [x] [BungeeCord](https://github.com/SpigotMC/BungeeCord)
  - [x] [BungeeGuard](https://github.com/lucko/BungeeGuard)
  - [x] [Velocity](https://github.com/PaperMC/Velocity)

<!-- Check out our [Github Project](https://github.com/orgs/Pumpkin-MC/projects/3) to see current progress. -->

## How to run

See our [Quick Start](https://docs.pumpkinmc.org/#quick-start) guide to get Pumpkin running.

## Contributions

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md)

## Docs

Pumpkin's documentation can be found at <https://pumpkinmc.org/>

## Communication

Consider joining [our Discord server](https://discord.gg/wT8XjrjKkf) to stay up-to-date on events, updates, and connect with other members.

## Funding

If you want to fund me and help the project, check out the [Donation Page](https://pumpkinmc.org/donate/).

## License & Attribution

* **Pumpkin Server**: Licensed under the [GNU General Public License v3.0 (GPLv3)](LICENSE).
* **Plugin API (`pumpkin-plugin-api` & `pumpkin-plugin-wit`)**: Dual-licensed under [MIT](crates/pumpkin-plugin-api/LICENSE-MIT) OR [Apache-2.0](crates/pumpkin-plugin-api/LICENSE-APACHE) for maximum flexibility when writing plugins.
* **Third-Party Assets & Data**: Bedrock mappings, protocol conversion data, and Minecraft assets are subject to their respective licenses and attribution terms. See [assets/NOTICE.md](assets/NOTICE.md) for full details.
