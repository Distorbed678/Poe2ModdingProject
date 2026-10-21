# Official mode: observing a session on the official servers

The client hook has a second mode besides the private server: **official mode**. It lets you play the real game on the official servers with the hook loaded, and the hook only *watches*: it logs the packets, the client's own state and a few snapshots to files on your machine. It is how this project compared the real server with its own. This guide says what it does, what it does not do, what risk you take, and the exact steps.

**Read the risk section first.** The mode is read-only for the game's state and for the servers, but it is not invisible: the hook is still a foreign DLL inside the game process. Nothing here is endorsed by Grinding Gear Games, and you use it at your own risk (see the [disclaimer](../../README.md#disclaimer)).

## What it does and does not do

| It does | It does not |
|---|---|
| Starts your own copy of the game (the `GameFiles/` copy, never the Steam install) with the original realm list, so the normal Steam login and the official realms work | Send anything to an official server that the game would not send itself |
| Records every login and instance message the game sends and receives (plaintext, read inside the client after decryption) to `packets.jsonl` and `packets.txt` | Change, inject, replay or block a packet |
| Writes logs, an optional per-frame movement log, and snapshots and area exports you ask for in the overlay | Move your character, press keys, or change game memory values |
| Shows an overlay (Insert) with the packets, coverage and a minimap; a badge `OFFICIAL: read-only` is always visible | Patch the login (the Steam login is untouched), bypass the key exchange, or use the debug link, position sync or terrain upload |
| Redacts the bodies of the login, credential and ticket messages before they are stored | Load mods (the mod loader does not even read the mods folder in this mode) |

### The read-only latch

In official mode the DLL sets its **read-only latch** at start and the latch never turns off for that process. Every part of the hook that could send, alter or inject anything checks it first: the crypto bypass, position sync, the side channel to a private server, the test-input and window-title hooks, the login patch, key injection and the mod loader. `login_mode` is forced to `steam` and `trusted_servers` to empty, both by the deploy and again when the DLL reads the config. The same latch also comes on in a private-mode client the first time it connects to an address that is not loopback or a trusted server, so a mistake in the config cannot turn a private client into a writer on the official servers. How it works and what is tested: [client-dll.md](../architecture/client-dll.md#modes-and-the-read-only-latch). At start the DLL writes a line to `poe-hook.log` that says the latch is on and what the guard state is.

## The risk (read this)

- **The DLL still patches game code in memory.** To observe, the hook installs inline detours on game functions, a Vulkan hook for the overlay, and it is loaded because it sits in the game folder under the name `version.dll`. "Read-only" means the game's state and the servers are not touched, not that the game process is unmodified.
- **It can be detected.** Anti-cheat and the game's terms of service can treat any injected DLL as a violation, even a passive one. The game can also load server-delivered code modules that inspect the process. There is no guarantee that this has not been, or will not be, noticed.
- **Your account is what is at risk**: a warning, a temporary or a permanent ban are possible. Do not use official mode on an account you care about, on an account with purchases, or on one you cannot afford to lose. A throw-away account is the safer choice, although the terms apply to it too.
- **Captures contain your play.** They hold what the server sent you: your character's data, inventory, chat and the names of other players near you. The login messages are redacted, nothing else is. Do not post a capture or log anywhere without reading it, and never attach one to a public issue.
- **The hook is built for one client build.** With an unverified build (the game was patched and the signature tables do not match) only the pure observers start. Use the build the repository names (`versions/0.5.5e.toml`) and update your `GameFiles/` copy first when the game is patched.
- The **launcher never offers this mode.** It is only available by hand, with `cargo xtask`.

If any of that is not acceptable to you, stop here: playing the game on the official servers through Steam's own, untouched install is unaffected by this project.

## Requirements

- A checkout of the main repository with its submodules and the tools ([setup.md](setup.md#clone), [setup.md](setup.md#tools)).
- A **copy** of the game files in `GameFiles/` that is the same build as the live game. Never point the tooling at Steam's own folder; the project never writes to the Steam install or to its Proton prefix.
- Linux: Steam running (the client exits at once without it) and a Proton installed through Steam; the copy runs in its own prefix (`third_party/proton-prefix`), so you sign in to Steam normally and the game asks for your usual Path of Exile 2 login.

## Step by step

1. **Deploy in official mode.**
   ```sh
   cargo xtask deploy-client --official
   ```
   This builds the DLLs (add `--no-build` to skip that), puts the original `gateway_list.txt` back from `gateway_list.txt.orig`, installs `version.dll` and `poe_hook_core.dll`, and writes `mode = "official"`, `login_mode = "steam"` and an empty `trusted_servers` into `poe-hook.toml`. Everything is inside `GameFiles/`; the originals are backed up first. The command ends with `official, read-only`.
2. **Start Steam**, then run the game from the copy:
   ```sh
   cargo xtask run-client
   ```
   The login screen is the normal Steam one. The overlay badge `OFFICIAL: read-only` and the line in `poe-hook.log` confirm the mode. **Insert** shows or hides the overlay; **Home** saves a snapshot.
3. **Play.** Nothing about playing changes.
4. **Close the game.** The files are complete when the process has ended.
5. **Find the output.** Per run, a session folder `GameFiles/logs/poe-hook/<date>_<time>_<pid>/` holds `packets.jsonl` and `packets.txt` (one record per message: direction, channel, opcode, name, size, hex), the logs and the snapshots. `GameFiles/poe-hook.log` is the DLL's own log, `GameFiles/logs/Client.txt` is the game's. To keep captures apart from generated data, `scripts/dumps/migrate.sh` copies packet sessions to `dumps/<version>/captures/{official,local}/` (git-ignored). Opening and comparing a capture: [captures.md](../re/captures.md).
6. **Switch back.**
   ```sh
   cargo xtask deploy-client      # private server again (gateway entry, standalone login)
   cargo xtask revert-client      # or: take the hook out of the copy altogether
   ```
   `deploy-client` without `--official` points the copy at the private server again (and resets the remembered realm for it); `revert-client` restores the original files and removes the hook ([client.md](client.md#going-back-to-the-official-game)).

## Settings that matter

`GameFiles/poe-hook.toml` has the rest of the hook's options (packet capture size and redaction, the movement log, the overlay keys). Official mode forces the safety-relevant ones; `redact` should stay on. The keys are described in [client-dll.md](../architecture/client-dll.md#login-mode-and-trusted-servers) and [the features section](../architecture/client-dll.md#features-features).

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| The client exits at once, log says `Unable to init Steam API` | Steam is not running |
| The login screen shows the private server's realm, or the account-name screen | `GameFiles/` was last deployed for the private server: run `deploy-client --official` again |
| The overlay is missing or features are off, `NOT verified` in the log | The game build differs from the one the hook knows: update the `GameFiles/` copy to the build in `versions/0.5.5e.toml`, or wait for a hook release for the new build ([new-game-version.md](new-game-version.md)) |
| The game freezes for seconds when monsters appear | A tracing option from an earlier private session is on (`entity_trace`); set it to `false` in `poe-hook.toml` |
| You were disconnected or warned | Stop using the mode and see the risk section; the project cannot help with an account decision |
