//! Private file protocol between the CUPS backend and the graphical prompt.
//!
//! Production installations provision a root-owned state layout for every
//! allowed user.  A dedicated `dupriman-<user>` group bridges CUPS's unprivileged
//! `lp` account and that one user without exposing job metadata to other users.
//! Direct single-user test runs retain a private 0700 directory layout.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptRequest {
    pub id: String,
    pub capability: String,
    pub job_id: String,
    pub user: String,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptResponse {
    pub id: String,
    pub capability: String,
    pub confirmed: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitOutcome {
    Confirmed,
    Declined(String),
    Timeout,
}

#[derive(Debug, Clone, Copy)]
pub struct UserIdentity {
    uid: u32,
    gid: u32,
}

#[derive(Debug, Clone)]
pub struct UserState {
    requests: PathBuf,
    responses: PathBuf,
    request_uid: u32,
    request_gid: Option<u32>,
    request_mode: u32,
    response_uid: u32,
    response_gid: Option<u32>,
    response_mode: u32,
}

impl UserState {
    pub fn requests_dir(&self) -> &Path {
        &self.requests
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn user_identity(user: &str) -> Result<UserIdentity> {
    if !valid_name(user) {
        bail!("invalid CUPS user name {user:?}");
    }
    let name = CString::new(user).expect("validated user name contains no NUL");
    let entry = unsafe { libc::getpwnam(name.as_ptr()) };
    if entry.is_null() {
        bail!("CUPS user {user:?} is not a local account");
    }
    Ok(UserIdentity {
        uid: unsafe { (*entry).pw_uid },
        gid: unsafe { (*entry).pw_gid },
    })
}

fn account_identity(name: &str) -> Result<(u32, u32)> {
    let name = CString::new(name).expect("literal contains no NUL");
    let entry = unsafe { libc::getpwnam(name.as_ptr()) };
    if entry.is_null() {
        bail!("required account {:?} does not exist", name);
    }
    Ok((unsafe { (*entry).pw_uid }, unsafe { (*entry).pw_gid }))
}

fn bridge_gid(user: &str) -> Result<u32> {
    let name = CString::new(format!("dupriman-{user}"))?;
    let entry = unsafe { libc::getgrnam(name.as_ptr()) };
    if entry.is_null() {
        bail!("required bridge group {:?} does not exist", name);
    }
    Ok(unsafe { (*entry).gr_gid })
}

fn euid() -> u32 {
    unsafe { libc::geteuid() }
}

fn mode_is(metadata: &fs::Metadata, mode: u32) -> bool {
    metadata.permissions().mode() & 0o7777 == mode
}

fn secure_state_root(root: &Path, identity: UserIdentity) -> Result<()> {
    let metadata = fs::metadata(root)
        .with_context(|| format!("cannot inspect state directory {}", root.display()))?;
    if !metadata.is_dir() {
        bail!("state directory {} is not a directory", root.display());
    }
    if metadata.permissions().mode() & 0o022 != 0 {
        bail!(
            "state directory {} must not be group/world writable",
            root.display()
        );
    }
    let valid_owner = if euid() == 0 {
        metadata.uid() == 0
    } else {
        metadata.uid() == 0 || metadata.uid() == identity.uid
    };
    if !valid_owner {
        bail!(
            "state directory {} must be owned by root or the submitting user",
            root.display(),
        );
    }
    Ok(())
}

fn chown(path: &Path, uid: u32, gid: u32) -> Result<()> {
    let path =
        CString::new(path.as_os_str().as_encoded_bytes()).expect("filesystem path contains no NUL");
    if unsafe { libc::chown(path.as_ptr(), uid, gid) } != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("cannot assign ownership of {}", path.to_string_lossy()));
    }
    Ok(())
}

fn secure_dir(path: &Path, uid: u32, gid: u32, mode: u32) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("cannot inspect {}", path.display()))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != uid
        || metadata.gid() != gid
        || !mode_is(&metadata, mode)
    {
        bail!("insecure prompt directory {}", path.display());
    }
    Ok(())
}

fn provision_dir(path: &Path, uid: u32, gid: u32, mode: u32) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => {
            chown(path, uid, gid)?;
            fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error).with_context(|| format!("cannot create {}", path.display()));
        }
    }
    secure_dir(path, uid, gid, mode)
}

fn direct_state(root: &Path, user: &str, identity: UserIdentity) -> Result<UserState> {
    let dir = root.join(user);
    match fs::symlink_metadata(&dir) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.uid() != identity.uid
                || !mode_is(&metadata, 0o700)
            {
                bail!(
                    "prompt directory {} must be private to {user}",
                    dir.display()
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && euid() == identity.uid => {
            fs::create_dir(&dir)
                .with_context(|| format!("cannot create prompt directory {}", dir.display()))?;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
        Err(error) => {
            return Err(error).with_context(|| format!("cannot inspect {}", dir.display()));
        }
    }
    Ok(UserState {
        requests: dir.clone(),
        responses: dir,
        request_uid: identity.uid,
        request_gid: None,
        request_mode: 0o600,
        response_uid: identity.uid,
        response_gid: None,
        response_mode: 0o600,
    })
}

fn bridge_state(root: &Path, user: &str, identity: UserIdentity) -> Result<UserState> {
    let bridge_gid = bridge_gid(user)?;
    let (cups_uid, _) = account_identity("lp")?;
    let user_dir = root.join(user);
    if euid() == 0 {
        provision_dir(&user_dir, 0, 0, 0o711)?;
        provision_dir(&user_dir.join("requests"), cups_uid, bridge_gid, 0o2750)?;
        provision_dir(
            &user_dir.join("responses"),
            identity.uid,
            bridge_gid,
            0o2770,
        )?;
    } else {
        secure_dir(&user_dir, 0, 0, 0o711)?;
        secure_dir(&user_dir.join("requests"), cups_uid, bridge_gid, 0o2750)?;
        secure_dir(
            &user_dir.join("responses"),
            identity.uid,
            bridge_gid,
            0o2770,
        )?;
    }
    Ok(UserState {
        requests: user_dir.join("requests"),
        responses: user_dir.join("responses"),
        request_uid: cups_uid,
        request_gid: Some(bridge_gid),
        request_mode: 0o640,
        response_uid: identity.uid,
        response_gid: Some(bridge_gid),
        response_mode: 0o640,
    })
}

pub fn user_state(root: &Path, user: &str) -> Result<UserState> {
    let identity = user_identity(user)?;
    secure_state_root(root, identity)?;
    let dir = root.join(user);
    match fs::symlink_metadata(&dir) {
        Ok(metadata) if metadata.uid() == 0 => bridge_state(root, user, identity),
        Ok(metadata) if metadata.uid() == identity.uid => direct_state(root, user, identity),
        Ok(_) => bail!("prompt directory {} has an unexpected owner", dir.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && euid() == 0 => {
            bridge_state(root, user, identity)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && euid() == identity.uid => {
            direct_state(root, user, identity)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(
            "prompt state for {user:?} is not provisioned; root must create the bridge layout"
        ),
        Err(error) => Err(error).with_context(|| format!("cannot inspect {}", dir.display())),
    }
}

pub fn request_path(state: &UserState, id: &str) -> PathBuf {
    state.requests.join(format!("req-{id}.json"))
}

pub fn response_path(state: &UserState, id: &str) -> PathBuf {
    state.responses.join(format!("resp-{id}.json"))
}

pub fn random_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn make_request_id(job_id: &str) -> Result<String> {
    Ok(format!("{job_id}-{}", &random_token()?[..16]))
}

fn secure_file(path: &Path, uid: u32, gid: Option<u32>, mode: u32) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("cannot inspect {}", path.display()))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.uid() != uid
        || gid.is_some_and(|gid| metadata.gid() != gid)
        || !mode_is(&metadata, mode)
    {
        bail!("rejected insecure prompt file {}", path.display());
    }
    Ok(())
}

/// Send a private request and return the recipient's state and identity.
pub fn send_request(root: &Path, req: &PromptRequest) -> Result<(UserState, UserIdentity)> {
    let identity = user_identity(&req.user)?;
    let state = user_state(root, &req.user)?;
    if euid() != 0 && euid() != state.request_uid {
        bail!(
            "backend user cannot write prompt requests for {:?}",
            req.user
        );
    }
    let path = request_path(&state, &req.id);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(state.request_mode)
        .open(&path)
        .with_context(|| format!("cannot create request {}", path.display()))?;
    file.write_all(&serde_json::to_vec_pretty(req)?)?;
    file.sync_all()?;
    drop(file);
    if euid() == 0 {
        chown(
            &path,
            state.request_uid,
            state.request_gid.unwrap_or(identity.gid),
        )?;
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(state.request_mode))?;
    secure_file(
        &path,
        state.request_uid,
        state.request_gid,
        state.request_mode,
    )?;
    Ok((state, identity))
}

pub fn wait_response(
    state: &UserState,
    id: &str,
    capability: &str,
    identity: UserIdentity,
    timeout: Option<Duration>,
    poll: Duration,
) -> Result<WaitOutcome> {
    let start = Instant::now();
    let path = response_path(state, id);
    loop {
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                secure_file(&path, identity.uid, state.response_gid, state.response_mode)?;
                let response: PromptResponse = serde_json::from_slice(&fs::read(&path)?)
                    .context("cannot parse prompt response")?;
                if response.id != id || response.capability != capability {
                    bail!("rejected prompt response with an invalid capability");
                }
                cleanup(state, id);
                return Ok(if response.confirmed {
                    WaitOutcome::Confirmed
                } else {
                    WaitOutcome::Declined(response.reason)
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("cannot inspect {}", path.display()));
            }
        }
        if let Some(timeout) = timeout
            && start.elapsed() >= timeout
        {
            cleanup(state, id);
            return Ok(WaitOutcome::Timeout);
        }
        sleep(poll);
    }
}

pub fn cleanup(state: &UserState, id: &str) {
    fs::remove_file(request_path(state, id)).ok();
    fs::remove_file(response_path(state, id)).ok();
}

pub fn pending_requests(state: &UserState) -> Vec<(PathBuf, PromptRequest)> {
    let mut items: Vec<(PathBuf, PromptRequest)> = fs::read_dir(state.requests_dir())
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("req-") && name.ends_with(".json"))
        })
        .filter(|path| {
            secure_file(
                path,
                state.request_uid,
                state.request_gid,
                state.request_mode,
            )
            .is_ok()
        })
        .filter_map(|path| {
            let request = serde_json::from_slice::<PromptRequest>(&fs::read(&path).ok()?).ok()?;
            Some((path, request))
        })
        .collect();
    items.sort_by(|left, right| left.0.cmp(&right.0));
    items
}

pub fn send_response(state: &UserState, response: &PromptResponse) -> Result<()> {
    if euid() != state.response_uid {
        bail!("only the submitting user may write a prompt response");
    }
    let path = response_path(state, &response.id);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(state.response_mode)
        .open(&path)
        .with_context(|| format!("cannot create response {}", path.display()))?;
    file.write_all(&serde_json::to_vec_pretty(response)?)?;
    file.sync_all()?;
    drop(file);
    fs::set_permissions(&path, fs::Permissions::from_mode(state.response_mode))?;
    secure_file(
        &path,
        state.response_uid,
        state.response_gid,
        state.response_mode,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current_user() -> String {
        std::env::var("USER").expect("tests require USER")
    }

    #[test]
    fn request_response_roundtrip_is_private() {
        let root = std::env::temp_dir().join(format!("dupriman-proto-{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        fs::create_dir(&root).unwrap();
        let user = current_user();
        let request = PromptRequest {
            id: "t1".into(),
            capability: random_token().unwrap(),
            job_id: "42".into(),
            user,
            title: "test document".into(),
            detail: "10 pages".into(),
        };
        let (state, identity) = send_request(&root, &request).unwrap();
        assert_eq!(
            fs::metadata(state.requests_dir())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(request_path(&state, &request.id))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(pending_requests(&state).len(), 1);
        send_response(
            &state,
            &PromptResponse {
                id: request.id.clone(),
                capability: request.capability.clone(),
                confirmed: false,
                reason: "cancelled".into(),
            },
        )
        .unwrap();
        let outcome = wait_response(
            &state,
            &request.id,
            &request.capability,
            identity,
            Some(Duration::from_secs(1)),
            Duration::from_millis(10),
        )
        .unwrap();
        assert_eq!(outcome, WaitOutcome::Declined("cancelled".into()));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn mismatched_capability_is_rejected() {
        let root = std::env::temp_dir().join(format!("dupriman-proto-cap-{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        fs::create_dir(&root).unwrap();
        let request = PromptRequest {
            id: "t2".into(),
            capability: random_token().unwrap(),
            job_id: "43".into(),
            user: current_user(),
            title: "test document".into(),
            detail: String::new(),
        };
        let (state, identity) = send_request(&root, &request).unwrap();
        send_response(
            &state,
            &PromptResponse {
                id: request.id.clone(),
                capability: "wrong".into(),
                confirmed: true,
                reason: String::new(),
            },
        )
        .unwrap();
        assert!(
            wait_response(
                &state,
                &request.id,
                &request.capability,
                identity,
                Some(Duration::from_secs(1)),
                Duration::from_millis(10)
            )
            .is_err()
        );
        fs::remove_dir_all(&root).ok();
    }
}
