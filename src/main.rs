use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use log::{debug, info, warn};
use mpd::idle::{Idle, Subsystem};
use mpd::{Client, Song, State};
use mpris_server::{Metadata, PlaybackStatus, Player, Time, TrackId};

#[derive(Clone)]
struct MpdConfig {
    addr: String,
    password: Option<String>,
}

#[derive(Debug)]
enum Command {
    Next,
    Previous,
    Pause,
    PlayPause,
    Stop,
    Play,
    SeekRelative(i64),
    SetPosition { track_id: String, position: i64 },
}

#[derive(Debug)]
enum WorkerEvent {
    Command(Command),
    Refresh,
}

#[derive(Debug)]
struct Snapshot {
    playback_status: PlaybackStatus,
    can_go_next: bool,
    can_go_previous: bool,
    can_seek: bool,
    position_micros: i64,
    metadata: Metadata,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    let local = tokio::task::LocalSet::new();
    local.run_until(run()).await
}

fn init_logging() {
    let env = env_logger::Env::default().default_filter_or("info");
    let mut builder = env_logger::Builder::from_env(env);
    builder.format_timestamp_secs();
    let _ = builder.try_init();
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = MpdConfig {
        addr: format!(
            "{}:{}",
            std::env::var("MPD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            std::env::var("MPD_PORT").unwrap_or_else(|_| "6600".to_string())
        ),
        password: std::env::var("MPD_PASSWORD").ok(),
    };

    let player = Player::builder("mpd")
        .identity("MPD")
        .can_control(true)
        .can_play(true)
        .can_pause(true)
        .can_go_next(true)
        .can_go_previous(true)
        .can_seek(true)
        .build()
        .await?;

    let (event_tx, event_rx) = mpsc::channel::<WorkerEvent>();
    let (snapshot_tx, mut snapshot_rx) = tokio::sync::mpsc::unbounded_channel::<Snapshot>();

    wire_controls(&player, event_tx.clone());
    spawn_worker(cfg, event_rx, event_tx, snapshot_tx);

    tokio::task::spawn_local(player.run());

    loop {
        tokio::select! {
            maybe_snapshot = snapshot_rx.recv() => {
                match maybe_snapshot {
                    Some(snapshot) => apply_snapshot(&player, snapshot).await?,
                    None => break,
                }
            }
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }
    }

    Ok(())
}

fn wire_controls(player: &Player, event_tx: mpsc::Sender<WorkerEvent>) {
    {
        let tx = event_tx.clone();
        player.connect_next(move |_| {
            let _ = tx.send(WorkerEvent::Command(Command::Next));
        });
    }
    {
        let tx = event_tx.clone();
        player.connect_previous(move |_| {
            let _ = tx.send(WorkerEvent::Command(Command::Previous));
        });
    }
    {
        let tx = event_tx.clone();
        player.connect_pause(move |_| {
            let _ = tx.send(WorkerEvent::Command(Command::Pause));
        });
    }
    {
        let tx = event_tx.clone();
        player.connect_play_pause(move |_| {
            let _ = tx.send(WorkerEvent::Command(Command::PlayPause));
        });
    }
    {
        let tx = event_tx.clone();
        player.connect_stop(move |_| {
            let _ = tx.send(WorkerEvent::Command(Command::Stop));
        });
    }
    {
        let tx = event_tx.clone();
        player.connect_play(move |_| {
            let _ = tx.send(WorkerEvent::Command(Command::Play));
        });
    }
    {
        let tx = event_tx.clone();
        player.connect_seek(move |_, offset| {
            let _ = tx.send(WorkerEvent::Command(Command::SeekRelative(
                offset.as_micros(),
            )));
        });
    }
    {
        let tx = event_tx;
        player.connect_set_position(move |_, track_id, position| {
            let _ = tx.send(WorkerEvent::Command(Command::SetPosition {
                track_id: track_id.to_string(),
                position: position.as_micros(),
            }));
        });
    }
}

fn spawn_worker(
    cfg: MpdConfig,
    event_rx: mpsc::Receiver<WorkerEvent>,
    event_tx: mpsc::Sender<WorkerEvent>,
    snapshot_tx: tokio::sync::mpsc::UnboundedSender<Snapshot>,
) {
    spawn_idle_listener(cfg.clone(), event_tx);

    thread::spawn(move || {
        loop {
            let mut client = match connect(&cfg) {
                Ok(client) => client,
                Err(err) => {
                    warn!("mpd connect error: {err}");
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
            };

            info!("mpd command/status connection established");
            let _ = send_snapshot(&mut client, &snapshot_tx);

            loop {
                let event = match event_rx.recv() {
                    Ok(event) => event,
                    Err(_) => return,
                };

                if let WorkerEvent::Command(cmd) = event
                    && let Err(err) = handle_command(&mut client, cmd)
                {
                    warn!("mpd command error: {err}");
                    break;
                }

                if let Err(err) = send_snapshot(&mut client, &snapshot_tx) {
                    if snapshot_tx.is_closed() {
                        return;
                    }
                    warn!("mpd refresh error: {err}");
                    break;
                }
            }
        }
    });
}

fn spawn_idle_listener(cfg: MpdConfig, event_tx: mpsc::Sender<WorkerEvent>) {
    thread::spawn(move || {
        loop {
            let mut client = match connect(&cfg) {
                Ok(client) => client,
                Err(err) => {
                    warn!("mpd idle connect error: {err}");
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
            };

            info!("mpd idle connection established");
            let _ = event_tx.send(WorkerEvent::Refresh);

            loop {
                match client.wait(&[
                    Subsystem::Player,
                    Subsystem::Queue,
                    Subsystem::Mixer,
                    Subsystem::Options,
                ]) {
                    Ok(_) => {
                        debug!("mpd idle event: refresh requested");
                        if event_tx.send(WorkerEvent::Refresh).is_err() {
                            return;
                        }
                    }
                    Err(err) => {
                        warn!("mpd idle wait error: {err}");
                        break;
                    }
                }
            }
        }
    });
}

fn send_snapshot(
    client: &mut Client,
    snapshot_tx: &tokio::sync::mpsc::UnboundedSender<Snapshot>,
) -> Result<(), mpd::error::Error> {
    let snapshot = take_snapshot(client)?;
    let _ = snapshot_tx.send(snapshot);
    Ok(())
}

fn connect(cfg: &MpdConfig) -> Result<Client, mpd::error::Error> {
    let mut client = Client::connect(&cfg.addr)?;
    if let Some(password) = &cfg.password {
        client.login(password)?;
    }
    Ok(client)
}

fn handle_command(client: &mut Client, cmd: Command) -> Result<(), mpd::error::Error> {
    debug!("mpris command: {:?}", cmd);
    match cmd {
        Command::Next => client.next(),
        Command::Previous => client.prev(),
        Command::Pause => client.pause(true),
        Command::PlayPause => client.toggle_pause(),
        Command::Stop => client.stop(),
        Command::Play => client.play(),
        Command::SeekRelative(offset_micros) => {
            let status = client.status()?;
            let current = status
                .elapsed
                .or_else(|| status.time.map(|(elapsed, _)| elapsed))
                .unwrap_or(Duration::ZERO)
                .as_secs_f64();
            let target = (current + offset_micros as f64 / 1_000_000.0).max(0.0);
            debug!(
                "seek relative: offset_us={} current_s={:.3} target_s={:.3}",
                offset_micros, current, target
            );
            client.rewind(target)
        }
        Command::SetPosition { track_id, position } => {
            let status = client.status()?;
            let current_track_id = track_path_for_status(&status);
            let accepts_any_track = track_id == "/";
            if accepts_any_track || current_track_id.as_ref() == Some(&track_id) {
                let target = (position as f64 / 1_000_000.0).max(0.0);
                debug!(
                    "set position accepted: track_id={} current_track_id={:?} target_s={:.3}",
                    track_id, current_track_id, target
                );
                client.rewind(target)?;
            } else {
                debug!(
                    "set position ignored: requested_track_id={} current_track_id={:?}",
                    track_id, current_track_id
                );
            }
            Ok(())
        }
    }
}

fn take_snapshot(client: &mut Client) -> Result<Snapshot, mpd::error::Error> {
    let status = client.status()?;
    let song = client.currentsong()?;

    let position_micros = duration_to_micros(
        status
            .elapsed
            .or_else(|| status.time.map(|(elapsed, _)| elapsed))
            .unwrap_or(Duration::ZERO),
    );
    let duration_micros = status
        .duration
        .or_else(|| status.time.map(|(_, total)| total))
        .or_else(|| song.as_ref().and_then(|s| s.duration))
        .map(duration_to_micros);

    Ok(Snapshot {
        playback_status: playback_status(status.state),
        can_go_next: status.nextsong.is_some(),
        can_go_previous: status.song.map(|s| s.pos > 0).unwrap_or(false),
        can_seek: duration_micros.is_some(),
        position_micros,
        metadata: metadata_for_song(
            track_path_for_status(&status),
            duration_micros,
            song.as_ref(),
        ),
    })
}

fn playback_status(state: State) -> PlaybackStatus {
    match state {
        State::Play => PlaybackStatus::Playing,
        State::Pause => PlaybackStatus::Paused,
        State::Stop => PlaybackStatus::Stopped,
    }
}

fn track_path_for_status(status: &mpd::Status) -> Option<String> {
    status
        .song
        .map(|place| format!("/io/github/darkwater/mpd/track/{}", place.id.0))
}

fn metadata_for_song(
    track_path: Option<String>,
    duration: Option<i64>,
    song: Option<&Song>,
) -> Metadata {
    let track_id = track_path
        .and_then(|path| TrackId::try_from(path).ok())
        .unwrap_or(TrackId::NO_TRACK);

    let mut builder = Metadata::builder().trackid(track_id);

    if let Some(length) = duration {
        builder = builder.length(Time::from_micros(length));
    }

    if let Some(song) = song {
        let title = song
            .title
            .clone()
            .or_else(|| song.name.clone())
            .unwrap_or_else(|| song.file.clone());
        builder = builder.title(title);

        if let Some(artist) = song.artist.clone() {
            builder = builder.artist([artist]);
        }

        if let Some(album) = song
            .tags
            .iter()
            .find_map(|(k, v)| if k == "Album" { Some(v.clone()) } else { None })
        {
            builder = builder.album(album);
        }

        if song.file.contains("://") {
            builder = builder.url(song.file.clone());
        }
    }

    builder.build()
}

fn duration_to_micros(duration: Duration) -> i64 {
    duration.as_micros().try_into().unwrap_or(i64::MAX)
}

async fn apply_snapshot(
    player: &Player,
    snapshot: Snapshot,
) -> Result<(), mpris_server::zbus::Error> {
    player.set_playback_status(snapshot.playback_status).await?;
    player.set_can_go_next(snapshot.can_go_next).await?;
    player.set_can_go_previous(snapshot.can_go_previous).await?;
    player.set_can_seek(snapshot.can_seek).await?;
    player.set_metadata(snapshot.metadata).await?;
    player.set_position(Time::from_micros(snapshot.position_micros));
    Ok(())
}
