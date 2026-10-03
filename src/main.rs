mod status;

use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, bail};
use reqwest::cookie::{CookieStore, Jar};
use status::StatusUpdate;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    process::Command,
    sync::mpsc,
    time::{self, Instant},
};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;
use url::Url;
use vrchatapi::{
    apis::{self, configuration::Configuration},
    models::{
        GetUser200Response, RegisterUserAccount200Response, TwoFactorAuthCode, TwoFactorAuthType,
        TwoFactorEmailCode, UpdateUserRequest,
    },
};

const API_ORIGIN: &str = "https://api.vrchat.cloud/";
const API_INTERVAL: Duration = Duration::from_secs(60);
const USER_AGENT: &str =
    "vrchat-status-helper/0.1.0 (https://github.com/surumeika1987/noctalia-vrchat-status-helper)";

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [] => {
            debug!(mode = "daemon", "starting helper");
            run_daemon().await
        }
        [command] if command == "login" => {
            debug!(mode = "login", "starting helper");
            login().await
        }
        [command] if command == "test" => {
            debug!(mode = "test", "starting helper");
            run_test_mode().await
        }
        [command, subcommand, payload] if command == "msg" && subcommand == "push-status" => {
            debug!(
                mode = "msg",
                message_len = payload.len().saturating_sub(2),
                "starting helper"
            );
            StatusUpdate::parse(payload)?;
            send_message(&format!("push-status {payload}")).await
        }
        [command, subcommand] if command == "msg" && subcommand == "request-push" => {
            debug!(mode = "msg", "requesting cached helper status");
            send_message("request-push").await
        }
        _ => bail!(
            "usage: vrchat-status-helper [login | test | msg push-status <status-number>:<message> | msg request-push]"
        ),
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn cache_file() -> Result<PathBuf> {
    let base = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".cache")))
        .context("could not determine cache directory")?;
    Ok(base.join("noctalia/vrchat-status/cookies.txt"))
}

fn socket_path() -> Result<PathBuf> {
    let runtime = env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    Ok(PathBuf::from(runtime).join("vrchat-status-helper.sock"))
}

fn configuration(jar: Arc<Jar>) -> Result<Configuration> {
    let client = reqwest::Client::builder().cookie_provider(jar).build()?;
    let mut config = Configuration::default();
    config.client = client.into();
    config.user_agent = Some(USER_AGENT.to_owned());
    Ok(config)
}

fn load_cookie_jar(path: &Path) -> Result<Arc<Jar>> {
    let jar = Arc::new(Jar::default());
    if path.exists() {
        debug!(path = %path.display(), "loading saved cookies");
        let raw = fs::read_to_string(path).context("failed to read cookie file")?;
        let origin = Url::parse(API_ORIGIN)?;
        for cookie in raw.lines().filter(|line| !line.trim().is_empty()) {
            jar.add_cookie_str(cookie, &origin);
        }
    } else {
        debug!(path = %path.display(), "cookie file does not exist");
    }
    Ok(jar)
}

fn save_cookies(path: &Path, jar: &Jar) -> Result<()> {
    let origin = Url::parse(API_ORIGIN)?;
    let cookies = jar
        .cookies(&origin)
        .context("VRChat did not return cookies")?;
    let value = cookies.to_str().context("cookie contains invalid text")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    for cookie in value.split(';') {
        writeln!(file, "{}", cookie.trim())?;
    }
    debug!(path = %path.display(), "saved cookies with restricted permissions");
    Ok(())
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}: ");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim_end().to_owned())
}

async fn login() -> Result<()> {
    info!("starting interactive VRChat login");
    let username = prompt("Username")?;
    let password = prompt("Password")?;
    let jar = Arc::new(Jar::default());
    let mut config = configuration(jar.clone())?;
    config.basic_auth = Some((username, Some(password)));

    let response = apis::authentication_api::get_current_user(&config).await?;
    match response {
        RegisterUserAccount200Response::CurrentUser(_) => {
            debug!("VRChat accepted credentials without additional authentication");
        }
        RegisterUserAccount200Response::RequiresTwoFactorAuth(requirement) => {
            debug!(methods = ?requirement.requires_two_factor_auth, "VRChat requested two-factor authentication");
            let code = prompt("Two-factor authentication code")?;
            if requirement
                .requires_two_factor_auth
                .contains(&TwoFactorAuthType::EmailOtp)
            {
                apis::authentication_api::verify2_fa_email_code(
                    &config,
                    TwoFactorEmailCode::new(code),
                )
                .await?;
            } else {
                apis::authentication_api::verify2_fa(&config, TwoFactorAuthCode::new(code)).await?;
            }
            match apis::authentication_api::get_current_user(&config).await? {
                RegisterUserAccount200Response::CurrentUser(_) => {}
                _ => bail!("two-factor authentication did not complete"),
            }
        }
    }
    save_cookies(&cache_file()?, jar.as_ref())?;
    info!("VRChat login completed");
    println!("Login succeeded; cookies were saved with mode 0600.");
    Ok(())
}

async fn authenticate_from_cookie(path: &Path) -> Result<(Configuration, String, StatusUpdate)> {
    debug!(path = %path.display(), "validating saved VRChat session");
    let jar = load_cookie_jar(path)?;
    let config = configuration(jar)?;
    match apis::authentication_api::get_current_user(&config).await? {
        RegisterUserAccount200Response::CurrentUser(user) => {
            debug!("saved VRChat session is valid");
            let current = StatusUpdate {
                status: user.status,
                message: user.status_description,
            };
            Ok((config, user.id, current))
        }
        _ => bail!("saved cookie requires two-factor authentication; run login"),
    }
}

async fn send_message(payload: &str) -> Result<()> {
    let path = socket_path()?;
    debug!(path = %path.display(), "connecting to helper daemon");
    let mut stream = UnixStream::connect(&path)
        .await
        .with_context(|| format!("daemon is not listening at {}", path.display()))?;
    stream.write_all(payload.as_bytes()).await?;
    stream.shutdown().await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    if response.trim() != "ok" {
        bail!("daemon rejected request: {}", response.trim());
    }
    debug!("daemon accepted status update request");
    Ok(())
}

#[derive(Debug)]
enum DaemonCommand {
    PushStatus(StatusUpdate),
    RequestPush,
}

fn parse_daemon_command(command: &str) -> Result<DaemonCommand> {
    if command == "request-push" {
        return Ok(DaemonCommand::RequestPush);
    }
    if let Some(payload) = command.strip_prefix("push-status ") {
        return Ok(DaemonCommand::PushStatus(StatusUpdate::parse(payload)?));
    }
    bail!("unknown helper IPC command")
}

struct SocketGuard(PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        debug!(path = %self.0.display(), "removing helper socket");
        let _ = fs::remove_file(&self.0);
    }
}

async fn bind_socket(path: &Path) -> Result<(UnixListener, SocketGuard)> {
    if path.exists() {
        if UnixStream::connect(path).await.is_ok() {
            bail!("another daemon is already listening at {}", path.display());
        }
        warn!(path = %path.display(), "removing stale helper socket");
        fs::remove_file(path).context("failed to remove stale socket")?;
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    info!(path = %path.display(), "helper socket is listening");
    Ok((listener, SocketGuard(path.to_owned())))
}

fn is_auth_error<T>(error: &apis::Error<T>) -> bool {
    matches!(
        error,
        apis::Error::ResponseError(response)
            if response.status == reqwest::StatusCode::UNAUTHORIZED
                || response.status == reqwest::StatusCode::FORBIDDEN
    )
}

fn status_from_user(response: GetUser200Response) -> StatusUpdate {
    match response {
        GetUser200Response::User(user) => StatusUpdate {
            status: user.status,
            message: user.status_description,
        },
        GetUser200Response::CurrentUser(user) => StatusUpdate {
            status: user.status,
            message: user.status_description,
        },
    }
}

async fn socket_server(listener: UnixListener, tx: mpsc::Sender<DaemonCommand>) -> Result<()> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        debug!("accepted helper IPC connection");
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut data = Vec::new();
            let response = match stream.read_to_end(&mut data).await {
                Ok(_) => {
                    let payload_len = data.len();
                    match String::from_utf8(data)
                        .ok()
                        .and_then(|s| parse_daemon_command(&s).ok())
                    {
                        Some(command) => {
                            debug!(command = ?command, "received helper IPC command");
                            if tx.send(command).await.is_ok() {
                                "ok\n"
                            } else {
                                "daemon stopping\n"
                            }
                        }
                        None => {
                            warn!(payload_len, "rejected invalid helper IPC payload");
                            "invalid payload\n"
                        }
                    }
                }
                Err(read_error) => {
                    warn!(error = %read_error, "failed to read helper IPC payload");
                    "read error\n"
                }
            };
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        });
    }
}

async fn notify_noctalia(update: &StatusUpdate) -> Result<()> {
    debug!(status = ?update.status, message_len = update.message.len(), "sending status to Noctalia");
    let status = Command::new("noctalia")
        .args([
            "msg",
            "plugin",
            "surumeika1987/vrchat-status:status",
            "all",
            "push-status",
            &update.payload(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;
    if !status.success() {
        bail!("noctalia IPC command failed with {status}");
    }
    debug!("Noctalia accepted status IPC command");
    Ok(())
}

fn cookie_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

fn advance_poll_deadline(next_poll: &mut Instant, now: Instant) {
    while *next_poll <= now {
        *next_poll += API_INTERVAL;
    }
}

fn update_api_schedule(
    next_poll: &mut Instant,
    api_started_at: Instant,
    api_completed_at: Instant,
) -> Instant {
    advance_poll_deadline(next_poll, api_completed_at);
    api_started_at + API_INTERVAL
}

async fn run_daemon() -> Result<()> {
    let cookie_path = cache_file()?;
    let socket_path = socket_path()?;
    let (listener, _guard) = bind_socket(&socket_path).await?;
    let (tx, mut rx) = mpsc::channel(16);
    tokio::spawn(async move {
        if let Err(error) = socket_server(listener, tx).await {
            error!(error = %error, "socket server stopped");
        }
    });

    let initial_api_started_at = Instant::now();
    let (mut session, initial_status) = match authenticate_from_cookie(&cookie_path).await {
        Ok((config, user_id, current)) => (Some((config, user_id)), Some(current)),
        Err(auth_error) => {
            warn!(error = %auth_error, "saved VRChat session is unavailable");
            (None, None)
        }
    };
    let mut observed_mtime = cookie_mtime(&cookie_path);
    let mut pending: Option<StatusUpdate> = None;
    let mut last_sent: Option<StatusUpdate> = None;
    let mut cached_status = initial_status;
    if let Some(current) = cached_status.clone() {
        info!(status = ?current.status, "initial VRChat status received during authentication");
        match notify_noctalia(&current).await {
            Ok(()) => last_sent = Some(current),
            Err(notify_error) => {
                warn!(error = %notify_error, "failed to send initial status to Noctalia");
            }
        }
    }
    let mut next_api = initial_api_started_at + API_INTERVAL;
    let mut next_poll = next_api;
    let mut next_login_notice = Instant::now();
    let mut tick = time::interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            Some(command) = rx.recv() => {
                match command {
                    DaemonCommand::PushStatus(update) => {
                        let replaced = pending.is_some();
                        debug!(status = ?update.status, message_len = update.message.len(), replaced, "queued latest status update");
                        pending = Some(update);
                    }
                    DaemonCommand::RequestPush => {
                        let current = match cached_status.clone() {
                            Some(status) => status,
                            None => StatusUpdate::parse("0:Need Login")?,
                        };
                        debug!(status = ?current.status, "sending cached status to Noctalia on request");
                        match notify_noctalia(&current).await {
                            Ok(()) => last_sent = Some(current),
                            Err(notify_error) => {
                                warn!(error = %notify_error, "failed to send requested status to Noctalia");
                            }
                        }
                    }
                }
            },
            _ = tick.tick() => {
                let now = Instant::now();
                if session.is_none() {
                    if now >= next_login_notice {
                        let mtime = cookie_mtime(&cookie_path);
                        if mtime != observed_mtime && now >= next_api {
                            debug!("cookie file changed; retrying VRChat authentication");
                            observed_mtime = mtime;
                            let api_started_at = Instant::now();
                            session = match authenticate_from_cookie(&cookie_path).await {
                                Ok((config, user_id, current)) => {
                                    info!(status = ?current.status, "VRChat session restored");
                                    cached_status = Some(current.clone());
                                    match notify_noctalia(&current).await {
                                        Ok(()) => last_sent = Some(current),
                                        Err(notify_error) => {
                                            warn!(error = %notify_error, "failed to send restored status to Noctalia");
                                        }
                                    }
                                    Some((config, user_id))
                                }
                                Err(auth_error) => {
                                    warn!(error = %auth_error, "VRChat reauthentication failed");
                                    None
                                }
                            };
                            next_api = api_started_at + API_INTERVAL;
                            if session.is_some() {
                                next_poll = next_api;
                            }
                        }
                        if session.is_none() {
                            debug!("notifying Noctalia that login is required");
                            let login_required = StatusUpdate::parse("0:Need Login")?;
                            match notify_noctalia(&login_required).await {
                                Ok(()) => last_sent = Some(login_required),
                                Err(notify_error) => {
                                    warn!(error = %notify_error, "failed to send login-required status to Noctalia");
                                }
                            }
                        }
                        next_login_notice = now + API_INTERVAL;
                    }
                    continue;
                }
                if now < next_api { continue; }

                let (config, user_id) = session.as_ref().unwrap();
                if let Some(update) = pending.take() {
                    debug!(status = ?update.status, message_len = update.message.len(), "updating VRChat status");
                    let request = UpdateUserRequest {
                        status: Some(update.status),
                        status_description: Some(update.message.clone()),
                        ..Default::default()
                    };
                    let api_started_at = Instant::now();
                    match apis::users_api::update_user(config, user_id, Some(request)).await {
                        Ok(_) => {
                            info!(status = ?update.status, "VRChat status update succeeded");
                            cached_status = Some(update.clone());
                            match notify_noctalia(&update).await {
                                Ok(()) => last_sent = Some(update.clone()),
                                Err(notify_error) => {
                                    warn!(error = %notify_error, "failed to mirror updated status to Noctalia");
                                }
                            }
                        }
                        Err(error) => {
                            warn!(error = %error, "VRChat status update failed; keeping latest request queued");
                            pending = Some(update);
                            if is_auth_error(&error) {
                                warn!("VRChat rejected the saved session; waiting for cookie replacement");
                                session = None;
                                cached_status = None;
                                observed_mtime = cookie_mtime(&cookie_path);
                                next_login_notice = Instant::now();
                            }
                        }
                    }
                    let api_completed_at = Instant::now();
                    next_api = update_api_schedule(
                        &mut next_poll,
                        api_started_at,
                        api_completed_at,
                    );
                } else if now >= next_poll {
                    debug!("fetching VRChat user status");
                    let api_started_at = Instant::now();
                    match apis::users_api::get_user(config, user_id).await {
                        Ok(response) => {
                            let current = status_from_user(response);
                            cached_status = Some(current.clone());
                            if last_sent.as_ref() != Some(&current) {
                                debug!(status = ?current.status, message_len = current.message.len(), "VRChat status changed");
                                match notify_noctalia(&current).await {
                                    Ok(()) => last_sent = Some(current),
                                    Err(notify_error) => {
                                        warn!(error = %notify_error, "failed to forward fetched status to Noctalia");
                                    }
                                }
                            } else {
                                debug!("VRChat status is unchanged");
                            }
                        }
                        Err(error) => {
                            warn!(error = %error, "VRChat status fetch failed");
                            if is_auth_error(&error) {
                                warn!("VRChat rejected the saved session; waiting for cookie replacement");
                                session = None;
                                cached_status = None;
                                observed_mtime = cookie_mtime(&cookie_path);
                                next_login_notice = Instant::now();
                            }
                        }
                    }
                    let api_completed_at = Instant::now();
                    next_api = update_api_schedule(
                        &mut next_poll,
                        api_started_at,
                        api_completed_at,
                    );
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("received shutdown signal");
                break;
            },
        }
    }
    info!("helper daemon stopped");
    Ok(())
}

fn apply_test_command(current: &mut StatusUpdate, command: DaemonCommand) -> Option<StatusUpdate> {
    match command {
        DaemonCommand::PushStatus(update) if update != *current => {
            *current = update.clone();
            Some(update)
        }
        DaemonCommand::PushStatus(_) => None,
        DaemonCommand::RequestPush => Some(current.clone()),
    }
}

async fn run_test_mode() -> Result<()> {
    let socket_path = socket_path()?;
    let (listener, _guard) = bind_socket(&socket_path).await?;
    let (tx, mut rx) = mpsc::channel(16);
    tokio::spawn(async move {
        if let Err(error) = socket_server(listener, tx).await {
            error!(error = %error, "test mode socket server stopped");
        }
    });

    let mut current = StatusUpdate::parse("4:Test Mode")?;
    info!(status = ?current.status, "test mode started");
    if let Err(error) = notify_noctalia(&current).await {
        warn!(error = %error, "failed to send initial test status to Noctalia");
    }

    loop {
        tokio::select! {
            Some(command) = rx.recv() => {
                if let Some(update) = apply_test_command(&mut current, command) {
                    debug!(status = ?update.status, message_len = update.message.len(), "sending test status to Noctalia");
                    if let Err(error) = notify_noctalia(&update).await {
                        warn!(error = %error, "failed to send test status to Noctalia");
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("received shutdown signal");
                break;
            }
        }
    }

    info!("helper test mode stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_command_parses_push_status() {
        match parse_daemon_command("push-status 3:Available now").unwrap() {
            DaemonCommand::PushStatus(update) => {
                assert_eq!(update.payload(), "3:Available now");
            }
            DaemonCommand::RequestPush => panic!("expected push-status"),
        }
    }

    #[test]
    fn daemon_command_parses_request_push_without_payload() {
        assert!(matches!(
            parse_daemon_command("request-push").unwrap(),
            DaemonCommand::RequestPush
        ));
        assert!(parse_daemon_command("request-push ignored").is_err());
        assert!(parse_daemon_command("3:legacy format").is_err());
    }

    #[test]
    fn api_limit_and_poll_cycle_use_call_start() {
        let api_started_at = Instant::now();
        let api_completed_at = api_started_at + Duration::from_secs(5);
        let mut next_poll = api_started_at;

        let next_api = update_api_schedule(&mut next_poll, api_started_at, api_completed_at);

        assert_eq!(next_api, api_started_at + API_INTERVAL);
        assert_eq!(next_poll, api_started_at + API_INTERVAL);
    }

    #[test]
    fn poll_deadline_keeps_its_original_cycle() {
        let scheduled = Instant::now() + API_INTERVAL;
        let mut next_poll = scheduled;

        advance_poll_deadline(&mut next_poll, scheduled + Duration::from_secs(1));

        assert_eq!(next_poll, scheduled + API_INTERVAL);
    }

    #[test]
    fn poll_deadline_skips_all_elapsed_cycles() {
        let scheduled = Instant::now() + API_INTERVAL;
        let mut next_poll = scheduled;
        let two_intervals = API_INTERVAL + API_INTERVAL;

        advance_poll_deadline(
            &mut next_poll,
            scheduled + two_intervals + Duration::from_secs(1),
        );

        assert_eq!(next_poll, scheduled + two_intervals + API_INTERVAL);
    }

    #[test]
    fn test_mode_updates_and_returns_changed_status() {
        let mut current = StatusUpdate::parse("4:Test Mode").unwrap();
        let changed = apply_test_command(
            &mut current,
            DaemonCommand::PushStatus(StatusUpdate::parse("2:Ask first").unwrap()),
        )
        .unwrap();

        assert_eq!(current.payload(), "2:Ask first");
        assert_eq!(changed.payload(), "2:Ask first");
    }

    #[test]
    fn test_mode_does_not_return_unchanged_status() {
        let mut current = StatusUpdate::parse("4:Test Mode").unwrap();
        let unchanged = apply_test_command(
            &mut current,
            DaemonCommand::PushStatus(StatusUpdate::parse("4:Test Mode").unwrap()),
        );

        assert!(unchanged.is_none());
        assert_eq!(current.payload(), "4:Test Mode");
    }

    #[test]
    fn test_mode_request_push_returns_current_status() {
        let mut current = StatusUpdate::parse("3:Available").unwrap();
        let requested = apply_test_command(&mut current, DaemonCommand::RequestPush).unwrap();

        assert_eq!(requested.payload(), "3:Available");
        assert_eq!(current.payload(), "3:Available");
    }
}
