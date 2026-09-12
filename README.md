# cosmic-ext-applet-now-playing (fork)

> **This is a fork of [AdityaHebballe/cosmic-ext-applet-now-playing](https://github.com/AdityaHebballe/cosmic-ext-applet-now-playing)**,
> an MPRIS now-playing applet for the COSMIC panel. All credit for the applet goes to the original author.
> The license is unchanged: **GPL-3.0-only** (see `LICENSE`).

## Changes in this fork

Maintained by [ReCosmicLabs](https://github.com/ReCosmicLabs) for the
[dotfiles](https://github.com/eualexandrerrr/dotfiles) setup. Everything below is a modification of the
original work, as required by section 5 of the GPL.

- **Player preference** (`src/player.rs`). A dedicated music player (Spotify) is picked whenever it has a
  track loaded, even if paused; browsers (Chromium, Chrome, Firefox, Brave, Vivaldi) are only used when
  nothing else is available. Upstream follows MPRIS' "whoever is playing" rule, which made a WhatsApp tab
  take over the panel.
- **Compact panel card** (`src/window.rs`). The panel shows the player's icon, a square cover, title and
  artist on two small lines (11 px / 9.5 px) and 14 px transport buttons, instead of one large text line.

The popup is unchanged.

---

Original README follows.

# cosmic-ext-applet-now-playing

A small COSMIC panel applet that shows what is currently playing via MPRIS.

It displays:
- Current track title and artist in the panel
- A popup with album art and media controls
- A compact MediaShell-inspired popup with square cover art and clear transport controls

![screenshot of the applet](./res/screenshot-1.png)
![screenshot of the applet 2](./res/screenshot-2.png)

## Build

```bash
just build-release
```

## Run (Local)

```bash
just run
```

## Install (System-Wide)

Build and install using the provided `just` recipes:

```bash
just build-release
sudo just install
```

To rebuild and reinstall after code changes:

```bash
just build-release && sudo just install
```

For a fully clean rebuild:

```bash
just clean && just build-release && sudo just install
```

Then add the applet from COSMIC panel settings.

## Feedback

Feedback is very welcome.

If you report an issue, please include:
- COSMIC version
- Distro and kernel
- Player app used
- Steps to reproduce
- Expected vs actual behavior

## License

This project is licensed under the [GPL-3.0-only license](./LICENSE)
