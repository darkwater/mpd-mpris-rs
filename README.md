# mpd-mpris-rs

mpd-mpris hammered my mpd and caused playback to stutter BIG, so i had codex
rewrite it in rust

it surprisingly made something functional in one prompt, and just a few prompts
later it seems to work perfectly with
[wayle](https://github.com/wayle-rs/wayle) which is cool

maybe i'll clean it up a bit later, haven't looked at the code myself yet

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
