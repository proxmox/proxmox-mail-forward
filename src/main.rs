//! A helper binary that forwards any mail passed via stdin to
//! the notification queue of the currently installed Proxmox product.
//!
//! If multiple products are installed side-by-side, only one notification is created.
//! The order of precedence is:
//!   - PVE
//!   - PBS
//!
//! The binary's path is added to /root/.forward, which means that
//! postfix will invoke it when the local root user receives an email message.
//! The message is passed via stdin.
//! The binary is installed with setuid permissions and will thus start as
//! root (euid ~ root, ruid ~ nobody).
//!
//! The setuid permissions are needed since postfix will run this binary as
//! `nobody`. If we would not become `root` via a setuid binary, we would need to
//! make the notification submission queue world-writable, which is
//! not what we want.
//!
//! Root privileges are only needed to switch to the unprivileged user that
//! submits to the notification queue of the installed product. Once the
//! product has been determined, the binary permanently drops to that user and
//! group before reading the mail from stdin.

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;
use anyhow::Error;
use anyhow::bail;
use nix::unistd::Gid;
use nix::unistd::Group;
use nix::unistd::Uid;
use nix::unistd::User;

use proxmox_log::LevelFilter;
use proxmox_log::Logger;
use proxmox_log::error;
use proxmox_notify::Notification;
use proxmox_notify::Severity;
use proxmox_notify::queue::QueueSubmitter;

enum ForwardError {
    Temporary(Error),
    Bounce(Error),
}

impl From<ForwardError> for ExitCode {
    fn from(value: ForwardError) -> Self {
        match value {
            // EX_TEMPFAIL -> postfix retries later
            ForwardError::Temporary(_) => ExitCode::from(75),
            // EX_DATAERR -> mail bounces
            ForwardError::Bounce(_) => ExitCode::from(65),
        }
    }
}

impl std::fmt::Display for ForwardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ForwardError::Temporary(error) => write!(
                f,
                "proxmox-mail-forward could not forward email to notification queue, retrying later: {error:#}"
            ),
            ForwardError::Bounce(error) => write!(
                f,
                "proxmox-mail-forward could not forward email, rejecting it: {error:#}"
            ),
        }
    }
}

/// A Proxmox product whose notifications are processed via a notification queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Product {
    /// Proxmox VE
    Pve,
    /// Proxmox Backup Server
    Pbs,
}

impl Product {
    /// All supported products, in order of precedence.
    const ALL: &[Product] = &[Product::Pve, Product::Pbs];

    /// Directory of the product's notification queue.
    fn queue_dir(self) -> &'static str {
        match self {
            Self::Pve => "/var/lib/pve-manager/notifications/queue",
            Self::Pbs => "/var/lib/proxmox-backup/notifications/queue",
        }
    }

    /// Check whether the product is installed on this host.
    ///
    /// A product counts as installed if its notification queue directory exists.
    fn is_installed(self) -> bool {
        Path::new(self.queue_dir()).exists()
    }

    /// Name of the unprivileged user that notifications are submitted as.
    fn user(self) -> &'static str {
        match self {
            Self::Pve => "www-data",
            Self::Pbs => "backup",
        }
    }

    /// Name of the unprivileged group that notifications are submitted as.
    fn group(self) -> &'static str {
        match self {
            Self::Pve => "www-data",
            Self::Pbs => "backup",
        }
    }

    /// Look up the ID of the user that notifications are submitted as.
    fn uid(self) -> Result<Uid, Error> {
        let user = self.user();

        let uid = User::from_name(user)
            .with_context(|| format!("could not look up username {user}"))?
            .with_context(|| format!("user {user} does not exist"))?
            .uid;

        Ok(uid)
    }

    /// Look up the ID of the group that notifications are submitted as.
    fn gid(self) -> Result<Gid, Error> {
        let group = self.group();

        let gid = Group::from_name(group)
            .with_context(|| format!("could not look up group {group}"))?
            .with_context(|| format!("group {group} does not exist"))?
            .gid;

        Ok(gid)
    }
}

/// Read data from stdin, until EOF is encountered.
fn read_stdin() -> Result<Vec<u8>, Error> {
    let mut input = Vec::new();
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();

    handle.read_to_end(&mut input)?;
    Ok(input)
}

fn installed_product() -> Result<Product, Error> {
    Product::ALL
        .iter()
        .copied()
        .find(|product| product.is_installed())
        .context("no supported Proxmox product installed")
}

/// Permanently switch to `uid` and `gid`, dropping all supplementary groups.
///
/// The real, effective and saved IDs are all set, so that the original privileges cannot be
/// regained afterwards.
fn drop_privileges(uid: Uid, gid: Gid) -> Result<(), Error> {
    // Supplementary groups and the group IDs can only be changed while still privileged, so the
    // user ID must be changed last.
    nix::unistd::setgroups(&[]).context("could not drop supplementary groups")?;
    nix::unistd::setresgid(gid, gid, gid).with_context(|| format!("could not set gid to {gid}"))?;
    nix::unistd::setresuid(uid, uid, uid).with_context(|| format!("could not set uid to {uid}"))?;

    if uid != Uid::from_raw(0) && nix::unistd::setuid(Uid::from_raw(0)).is_ok() {
        bail!("root privileges could be regained after dropping them");
    }

    Ok(())
}

fn forward_mail_from_stdin() -> Result<(), ForwardError> {
    let product = installed_product().map_err(ForwardError::Temporary)?;
    drop_privileges(
        product.uid().map_err(ForwardError::Temporary)?,
        product.gid().map_err(ForwardError::Temporary)?,
    )
    .map_err(ForwardError::Temporary)?;

    let mail = read_stdin()
        .context("could not read mail from STDIN")
        .map_err(ForwardError::Bounce)?;

    let queue = QueueSubmitter::new(product.queue_dir());

    let notification = Notification::forwarded_mail_builder(mail)
        .severity(Severity::Unknown)
        .field("hostname", proxmox_sys::nodename())
        .field("type", "system-mail")
        .build()
        .map_err(|err| ForwardError::Bounce(err.into()))?;

    queue
        .submit(&notification)
        .context("could not submit mail to notification queue")
        .map_err(ForwardError::Temporary)?;

    Ok(())
}

fn main() -> ExitCode {
    if let Err(err) = Logger::from_env("PROXMOX_LOG", LevelFilter::INFO)
        .journald()
        .init()
    {
        eprintln!("unable to initialize logging: {err}");
    }

    if let Err(err) = forward_mail_from_stdin() {
        error!("{err:#}");

        // Print the error to stderr as well, since that will be shown
        // in `mailq` output for deferred emails.
        eprintln!("{err:#}");

        return err.into();
    }

    ExitCode::SUCCESS
}
