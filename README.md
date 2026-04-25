# mpd-mpris-rs

mpd-mpris hammered my mpd and caused playback to stutter BIG, so i had codex
rewrite it in rust

it surprisingly made something functional in one prompt, and just a few prompts
later it seems to work perfectly with
[wayle](https://github.com/wayle-rs/wayle) which is cool

maybe i'll clean it up a bit later, haven't looked at the code myself yet

(rest of readme also written by codex)

## Usage

- Run the bridge:

```bash
cargo run --release
```

- Optional environment variables:
  - `MPD_HOST` (default: `127.0.0.1`)
  - `MPD_PORT` (default: `6600`)
  - `MPD_PASSWORD` (optional)
  - `RUST_LOG` for logs (example: `RUST_LOG=mpd_mpris_rs=debug`)

Example:

```bash
MPD_HOST=192.168.1.10 MPD_PORT=6600 RUST_LOG=info cargo run --release
```

## systemd user service

Install the program:

```bash
# after cloning
cargo install --path=.

# without cloning
cargo install --git=https://github.com/darkwater/mpd-mpris-rs
```

Install the included unit file:

```bash
# after cloning
install -Dm644 mpd-mpris-rs.service ~/.config/systemd/user/mpd-mpris-rs.service

# else just copy its contents to a file at that path manually
```

Enable and start it:

```bash
systemctl --user daemon-reload
systemctl --user enable --now mpd-mpris-rs.service
```

Check status/logs:

```bash
systemctl --user status mpd-mpris-rs.service
journalctl --user -u mpd-mpris-rs.service -f
```

One way to set environment variables for the user service:

```bash
systemctl --user edit mpd-mpris-rs.service
```

```systemd
[Service]
Environment=MPD_HOST=192.168.0.106
```
