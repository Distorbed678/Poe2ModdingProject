# Poe2 server list

The public list of private **Path of Exile 2** servers that the [Poe2 launcher](https://github.com/Distorbed678/Poe2Launcher) shows under *Servers > Browse*. The list is one file, [`servers.json`](servers.json); anyone adds a server with a pull request.

The launcher reads the file from `https://raw.githubusercontent.com/Distorbed678/Poe2ServerList/main/servers.json`. For every entry it then asks the server itself (`GET <address>/api/v1/server`, no login) for the players online, the client version and the ping, so what players see is live, not what the entry claims. The list says who the server is; the server says how it is doing.

> This is an unofficial fan project for the private server of Poe2ModdingProject. It is not connected to Grinding Gear Games. Listing a server here is not an endorsement or a review: the operator runs it, and players decide whether to trust it (a server sees the account names and passwords players type into it, over the address given below).

## List your server

1. Run a server that players can reach from the internet: the web API port (default 8080) and the game ports ([hosting guide](https://github.com/Distorbed678/Poe2ModdingProject/blob/main/docs/guides/hosting.md)). **Use https** for the web API if you can (a TLS reverse proxy in front); players are warned about plain `http://` before they add a server, and many will not.
2. Check that `https://<your address>/api/v1/server` answers with JSON.
3. Fork this repository and add one entry to the `servers` list in `servers.json` (copy it from [`servers.example.json`](servers.example.json)).
4. Run `python3 scripts/validate.py` (Python 3, nothing to install). Fix what it reports.
5. Open a pull request. A short description of the server is enough. The check in the pull request runs the same script.

To change or remove your entry, send another pull request. The `id` never changes; it is the key of your entry.

### The entry

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | Stable key, 3-48 characters of `a-z`, `0-9` and `-`. Pick it once. |
| `name` | yes | Shown to players, up to 64 characters. |
| `address` | yes | The address of the web API, the same text a player types under *Servers > Add a server*, for example `https://play.example.org` or `https://play.example.org:8443`. No path, no query. Must be reachable from the internet: `localhost`, `192.168.x.x` and other private addresses are refused. |
| `description` | no | Up to 400 characters, plain text. |
| `region` | no | A short region name: `EU`, `NA`, `Asia`, `OCE`, `SA`, ... up to 24 characters. |
| `game_version` | yes | The client version the server runs, for example `0.5.5e`. Players filter by it (the launcher shows the version the server reports when it answers). |
| `tags` | no | Up to 8 of `a-z`, `0-9`, `-` (24 characters each), for example `pve`, `pvp`, `hardcore`, `seasonal`, `friendly`. |
| `contact` | no | A Discord invite, a web page (an `https://` link becomes clickable) or an e-mail address, up to 120 characters. |
| `mods` | no | Mods that players need on their side, up to 16: `{"id": "core-lib", "version": "^1.2", "url": "https://..."}`; only `id` is required and `url` must be `https`. The launcher shows them; it does not install mods for servers yet. |

[`servers.schema.json`](servers.schema.json) is the same description as a JSON Schema (for editors). The file format has a `schema` number (now `1`); a launcher that does not know a newer number says so and shows the list it saved before.

### Rules

- One entry per server address. Duplicates are refused.
- Only servers that exist and answer. An entry that has been unreachable for a long time may be removed.
- No misleading names: do not pretend to be another operator or the official game.
- No tracking: an entry is data, never a link that must be opened.

## Maintainers

- Merge a pull request only when the `validate` check is green and the address answers.
- The launcher is forgiving: an invalid entry is skipped on the player's side and the rest of the list still works, so a mistake that slips through never breaks the list for everybody.
- Creating this repository: copy the contents of `docs/server-list-template/` of Poe2ModdingProject into the root of `Distorbed678/Poe2ServerList` (the `servers.json` must be at the root of the `main` branch).

## License

The list is public data. The scripts are MIT licensed.
