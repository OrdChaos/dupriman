//! Private file protocol between the CUPS backend and the graphical prompt.
//!
//! The state root is owned by root (or by the submitting user for direct test
//! runs).  Each CUPS user receives a private 0700 subdirectory.  Requests and
//! responses are 0600 and the backend verifies the response owner and a random
//! capability before printing backs.

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

fn user_identity(user: &str) -> Result<UserIdentity> {
    if user.is_empty()
        || !user
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        bail!("invalid CUPS user name {user:?}");
    }
    let name = CString::new(user).expect("validated user name contains no NUL");
    let entry = unsafe { libc::getpwnam(name.as_ptr()) };
    if entry.is_null() {
        bail!("CUPS user {user:?} is not a local account");
    }
    let entry = unsafe { &*entry };
    Ok(UserIdentity {
        uid: entry.pw_uid,
        gid: entry.pw_gid,
    })
}

fn euid() -> u32 {
    unsafe { libc::geteuid() }
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

fn chown(path: &Path, identity: UserIdentity) -> Result<()> {
    let path =
        CString::new(path.as_os_str().as_encoded_bytes()).expect("filesystem path contains no NUL");
    if unsafe { libc::chown(path.as_ptr(), identity.uid, identity.gid) } != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("cannot assign {} to prompt user", path.to_string_lossy()));
    }
    Ok(())
}

pub fn user_dir(root: &Path, user: &str) -> Result<PathBuf> {
    let identity = user_identity(user)?;
    secure_state_root(root, identity)?;
    let dir = root.join(user);
    match fs::symlink_metadata(&dir) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                bail!("prompt directory {} is not a real directory", dir.display());
            }
            if metadata.uid() != identity.uid || metadata.permissions().mode() & 0o077 != 0 {
                bail!(
                    "prompt directory {} must be private to {user}",
                    dir.display()
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&dir)
                .with_context(|| format!("cannot create prompt directory {}", dir.display()))?;
            if euid() == 0 {
                chown(&dir, identity)?;
            } else if euid() != identity.uid {
                bail!("backend must run as root or as the submitting user");
            }
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
        Err(error) => {
            return Err(error).with_context(|| format!("cannot inspect {}", dir.display()));
        }
    }
    Ok(dir)
}

pub fn request_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("req-{id}.json"))
}

pub fn response_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("resp-{id}.json"))
}

pub fn random_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn make_request_id(job_id: &str) -> Result<String> {
    Ok(format!("{job_id}-{}", &random_token()?[..16]))
}

/// Send a private request and return the recipient's directory and identity.
pub fn send_request(root: &Path, req: &PromptRequest) -> Result<(PathBuf, UserIdentity)> {
    let identity = user_identity(&req.user)?;
    let dir = user_dir(root, &req.user)?;
    let path = request_path(&dir, &req.id);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("cannot create request {}", path.display()))?;
    file.write_all(&serde_json::to_vec_pretty(req)?)?;
    file.sync_all()?;
    drop(file);
    if euid() == 0 {
        chown(&path, identity)?;
    } else if euid() != identity.uid {
        bail!("backend must run as root or as the submitting user");
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok((dir, identity))
}

pub fn wait_response(
    dir: &Path,
    id: &str,
    capability: &str,
    identity: UserIdentity,
    timeout: Option<Duration>,
    poll: Duration,
) -> Result<WaitOutcome> {
    let start = Instant::now();
    let path = response_path(dir, id);
    loop {
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.file_type().is_file()
                    || metadata.uid() != identity.uid
                    || metadata.permissions().mode() & 0o777 != 0o600
                {
                    bail!("rejected insecure prompt response {}", path.display());
                }
                let response: PromptResponse = serde_json::from_slice(&fs::read(&path)?)
                    .context("cannot parse prompt response")?;
                if response.id != id || response.capability != capability {
                    bail!("rejected prompt response with an invalid capability");
                }
                cleanup(dir, id);
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
            cleanup(dir, id);
            return Ok(WaitOutcome::Timeout);
        }
        sleep(poll);
    }
}

pub fn cleanup(dir: &Path, id: &str) {
    fs::remove_file(request_path(dir, id)).ok();
    fs::remove_file(response_path(dir, id)).ok();
}

pub fn pending_requests(dir: &Path) -> Vec<(PathBuf, PromptRequest)> {
    let mut items: Vec<(PathBuf, PromptRequest)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("req-") && name.ends_with(".json"))
        })
        .filter_map(|path| {
            let request = serde_json::from_slice::<PromptRequest>(&fs::read(&path).ok()?).ok()?;
            Some((path, request))
        })
        .collect();
    items.sort_by(|left, right| left.0.cmp(&right.0));
    items
}

pub fn send_response(dir: &Path, response: &PromptResponse) -> Result<()> {
    let path = response_path(dir, &response.id);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("cannot create response {}", path.display()))?;
    file.write_all(&serde_json::to_vec_pretty(response)?)?;
    file.sync_all()?;
    drop(file);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    fs::remove_file(request_path(dir, &response.id)).ok();
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
        let (dir, identity) = send_request(&root, &request).unwrap();
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(request_path(&dir, &request.id))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(pending_requests(&dir).len(), 1);
        send_response(
            &dir,
            &PromptResponse {
                id: request.id.clone(),
                capability: request.capability.clone(),
                confirmed: false,
                reason: "cancelled".into(),
            },
        )
        .unwrap();
        let outcome = wait_response(
            &dir,
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
        let (dir, identity) = send_request(&root, &request).unwrap();
        send_response(
            &dir,
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
                &dir,
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
