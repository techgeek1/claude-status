# Claude Status

A COSMIC panel applet that shows your Claude subscription usage limits and service status at a glance.

## Features

- **Usage monitoring** — displays 5-hour, 7-day, and Sonnet quota utilization with progress bars and reset countdowns
- **Extra usage tracking** — shows pay-as-you-go spend and budget when enabled
- **Service status** — per-component health from the Claude status page (claude.ai, API, Claude Code, etc.)
- **Active incidents** — surfaces ongoing incidents with the latest update
- **Panel icon indicator** — status dot overlay on the icon when services are degraded
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

Service status is fetched from the public Atlassian Statuspage API at `status.claude.com/api/v2/summary.json`. No authentication required.

## License

MIT
