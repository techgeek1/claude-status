# Claude Status

A COSMIC panel applet that shows your Claude subscription usage limits and service status at a glance.

## Features

- **Usage monitoring** — a progress bar with reset countdown for every quota the API reports (session, weekly, and per-model windows such as Fable), driven by the server's own limit list rather than a hardcoded set
- **Extra usage tracking** — shows pay-as-you-go spend and budget when enabled
- **Service status** — per-component health from the Claude status page (claude.ai, API, Claude Code, etc.)
- **Active incidents** — surfaces ongoing incidents with the latest update
- **Panel icon indicator** — status dot overlay on the icon when services are degraded
- **Remote-control sleep inhibitor** — optionally holds a logind `sleep:idle` inhibitor while a Claude Code remote-control session is attached, so a remote session isn't cut off by idle suspend (toggleable in the popup, on by default)
- **Auto-refresh** — status polled every 5 minutes in the background, usage + status refreshed every 60 seconds while the popup is open (debounced)

## Requirements

- COSMIC desktop environment (1.0.x)
- Claude Code installed and authenticated (the applet reads the OAuth token from `~/.claude/.credentials.json`)

## Building

```sh
cargo build --release
```

## Installing

```sh
just install
```

Then add "Claude Status" to your panel via COSMIC Settings > Desktop > Panel > Applets.

## Uninstalling

```sh
just uninstall
```

## How it works

Usage data is fetched from the (undocumented) OAuth usage endpoint at `api.anthropic.com/api/oauth/usage` using the access token that Claude Code manages. The applet does not handle token refresh — if the token expires, usage fetches gracefully degrade until Claude Code refreshes it.

Usage bars come from the response's `limits` array, which labels each window by kind and scope — so new model-scoped limits appear automatically. The older named fields (`five_hour`, `seven_day`, ...) are used as a fallback if that array is absent.

Remote-control detection watches `~/.claude/sessions/` with inotify (falling back to 10s polling). Claude Code merges a `bridgeSessionId` into `<pid>.json` while a remote client is attached and writes it back as `null` on detach; a session counts as attached when that field is set and the owning pid is alive. The inhibitor is a `login1.Manager.Inhibit("sleep:idle")` lock, held via the returned fd for as long as any session is attached. The preference is stored with cosmic-config under `dev.techgeek1.CosmicExtAppletClaudeStatus/v1`.

Service status is fetched from the public Atlassian Statuspage API at `status.claude.com/api/v2/summary.json`. No authentication required.

## License

MIT
