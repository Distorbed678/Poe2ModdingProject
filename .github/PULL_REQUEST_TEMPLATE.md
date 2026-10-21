## What and why

<!-- A short description of the change and the reason. Link the issue it closes, if any. -->

## Checks

- [ ] I ran: `cargo build && cargo test`, `python3 scripts/check-links.py`, `cargo xtask openapi --check` (when the web API changed), `cargo xtask tracker --check`
- [ ] Docs updated in the same change (`docs/`, READMEs, `docs/re/` for reverse-engineering findings)
- [ ] No game files, official-server captures, `Client.txt` excerpts, secrets or personal data in the change
- [ ] Nothing in the change sends, alters or injects anything on official servers (the read-only rule)
- [ ] I did not copy code from AGPL reference projects (KiwiNet, exileSniffer, LibGGPK3)
