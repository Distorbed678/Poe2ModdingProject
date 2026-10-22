//! The server's mod host (decisions: docs/plans/server-modding-analysis.md, guide:
//! docs/guides/modding.md, design: docs/architecture/mod-host.md).
//!
//! - [`store`]: the `mods/` folder (`state.toml`, one folder per mod, `packages/`), install,
//!   enable and disable, reproducible packing.
//! - [`ModHost`]: the load plan (manifest v2 through `poe-hook-mods`, server API and game
//!   version checks, dependencies), native libraries (`libloading`, `poe-mod-api`), data-tier
//!   commands and overlays, event delivery with read-only snapshots, the commands a mod
//!   submits, failure isolation.
//! - [`check`]: the login comparison of the client's reported mods with the required set.
//!
//! The host knows nothing about sockets or the world: the instance server turns its state into
//! snapshots, calls [`ModHost::dispatch`] and applies the returned [`ModCommand`]s on the task
//! that owns what they change.

pub mod check;
mod ffi;
mod host;
pub mod store;

pub use check::{RequiredMod, Verdict, check_client};
pub use ffi::{MAX_COMMANDS_PER_MOD, MAX_SUBMITTED, ModCommand};
pub use host::{
    CommandAction, CommandInfo, EventData, HostConfig, InstanceSnap, ModHost, PlayerSnap, Submitted,
};
pub use poe_mod_api::{
    COMMAND_BROADCAST,
    COMMAND_KICK,
    COMMAND_SEND_CHAT,
    COMMAND_TELEPORT,
    Command,
    CommandSpec,
    EVENT_COMMAND,
    EVENT_INSTANCE_CLOSED,
    EVENT_INSTANCE_CREATED,
    EVENT_PLAYER_JOIN,
    EVENT_PLAYER_LEAVE,
    EVENT_SERVER_START,
    EVENT_SERVER_STOP,
    EVENT_TICK,
    EVENTS_ALL,
    // For mods written as Rust functions (tests, embedding): the ABI types `from_inits` takes.
    Event,
    ModInfo,
    ROLE_ADMIN,
    ROLE_PLAYER,
    SERVER_API_VERSION,
    ServerApi,
    Str,
    event_bit,
};
