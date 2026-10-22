# Open items (2026-10-22)

Everything that is still to do outside the terrain port ([terrain-all-areas.md](terrain-all-areas.md)), in one list. Status per feature stays in the [README tracker](../../README.md#status); the history of how each item came about is in [handoff-orchestrator-3.md](handoff-orchestrator-3.md).

## Needs the user

| Item | What | Where |
|---|---|---|
| End-to-end test | launcher v0.4.0 AppImage: install the game, get the hook, Host (server update from server-v0.1.0 to 0.2.1), Play, web admin, restart | [guides/launcher.md](../guides/launcher.md#end-to-end-test-on-a-real-machine) |
| G-PARTY capture | official capture where a friend answers a party invite and a trade request; implementing accept and decline is deferred (below) | [protocol/entities.md](../protocol/entities.md#party-and-trade-flows-wp-c3) |
| Movement problem 5 | same capture session: sprint ended with W held since before the roll (client coasts 30 frames and resumes) | [protocol/movement.md](../protocol/movement.md) |
| Windows | launcher installer and update, Host a server (zip, `taskkill`), Play, restart under the launcher; needs a Windows machine or VM | [guides/launcher.md](../guides/launcher.md) |

## Can be done without the user

| Item | What | Where |
|---|---|---|
| Mod overlays | data-tier mods: apply `config_defaults` and server `opcodes` overlays | [architecture/mod-host.md](../architecture/mod-host.md) |
| Web admin Players page | show class names | modules/web-admin |
| Ban commands | bans from in-game chat and the server CLI (today only the web admin) | [guides/server-admin.md](../guides/server-admin.md) |
| Mod download rate limit | `GET /api/v1/mods/{id}/{version}.zip` has none | modules/server |
| Launcher nested hook pin | `modules/launcher/client-hook` is at 875afe4 (hook-v0.2.0+3), not hook-v0.3.0; harmless (poe-hook-mods is the same), bump with the next launcher change | modules/launcher |
| Movement gaps #11 and #12 | the tracker row "Server-authoritative movement" still lists them; the Sprint and Dodge roll rows say both were closed in WP C1, only problem 5 remains: reconcile the rows | [protocol/movement.md](../protocol/movement.md) |

## Deferred until items and entities are implemented (user decision 2026-10-22)

Not to be worked on before the item and entity systems exist.

| Item | Why it waits |
|---|---|
| Remote players' equipment visuals | needs items |
| Dynamic obstacles (entities) in the ported client pathfinder | needs entities |
| Party and trade accept/decline (after the G-PARTY capture) | trade needs inventories and items; party is entity-adjacent. The capture can still be taken when convenient |
