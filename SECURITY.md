# Security policy

## Reporting a vulnerability

Please report security problems **privately**, not in a public issue: use GitHub's private vulnerability reporting for this repository (the **Security** tab, then **Report a vulnerability**). If that button is not available, open a public issue that only says you have a security report and ask for a private way to send it; do not put details in the issue.

Include what you found, how to reproduce it (a version or commit, a config, steps), and what you think the impact is. This is a small volunteer project: expect an answer within about a week and a fix as time allows. Reporters are credited unless they prefer not to be.

## Scope

In scope for **Poe2ModdingProject**: the server (`poe-server`, the login and instance servers, the web API and its token handling, accounts and passwords, the rate limiter), the xtask and deploy tooling, and the release packages.

Out of scope: Grinding Gear Games' game, services and anti-cheat; bugs in Path of Exile 2 itself; using the project against official servers (not supported); problems that need a modified client or physical access to your machine and do not cross a trust boundary the documentation promises.

## Supported versions

The project follows **Path of Exile 2 client 0.5.5e**. Only the latest commit on `main` (and the latest release, if there is one) gets fixes.

## Notes for users

This is an unofficial hobby project for personal and educational use. It has not had a professional security audit; an internal pre-release review is in the main repository ([docs/plans/public-release-audit.md](https://github.com/Distorbed678/Poe2ModdingProject/blob/main/docs/plans/public-release-audit.md)). Hosting a server for other people is your responsibility: follow [docs/guides/hosting.md](docs/guides/hosting.md) (keep `dev_machine_hash_login` off, put the web API behind TLS).
