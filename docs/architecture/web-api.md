# Web administration and API

Planned, nothing built yet. The work packages are in [../plans/orchestration-2026-10.md](../plans/orchestration-2026-10.md) (phase 4).

## Today

Administration is by server commands, run from chat (`!` prefix) or the overlay's Commands page, with admins by account role and an audit log: [../guides/server-admin.md](../guides/server-admin.md). They go through one command registry (`servers/instance-server/src/commands.rs`), which is what the web API would reuse. The overlay's debug link is loopback only ([../guides/hosting.md](../guides/hosting.md)).

## Planned

- **Web API:** a crate inside the server (`poe-web-api`, axum: REST and WebSocket) for status (online players, instances, live log), instances (players, close, transfer), accounts (list, disable, role) and the registry's commands. Authentication uses the account model in [accounts.md](accounts.md); the API types come from an OpenAPI spec, which the launcher reuses.
- **Web admin:** a separate Svelte single-page app (Poe2WebAdmin repo) that the server can serve as static files: login, dashboard, live log viewer, instances, accounts, characters.
