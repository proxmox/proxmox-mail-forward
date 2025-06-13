//! A helper binary that forwards any mail passed via stdin to
//! proxmox_notify.
//!
//! The binary's path is added to /root/.forward, which means that
//! postfix will invoke it when the local root user receives an email message.
//! The message is passed via stdin.
//! The binary is installed with setuid permissions and will thus run as
//! root (euid ~ root, ruid ~ nobody)
//!
//! The forwarding behavior is the following:
//!   - PVE installed: Use PVE's notifications.cfg
//!   - PBS installed: Use PBS's notifications.cfg if present. If not,
//!     use an empty configuration and add a default sendmail target and
//!     a matcher - this is needed because notifications are not yet
//!     integrated in PBS.
//!   - PVE/PBS co-installed: Use PVE's config *and* PBS's config, but if
//!     PBS's config does not exist, a default sendmail target will *not* be
//!     added. We assume that PVE's config contains the desired notification
//!     behavior for system mails.
//!
use std::io::Read;
use std::path::Path;

use anyhow::Error;

use proxmox_log::LevelFilter;
use proxmox_log::Logger;
use proxmox_log::error;
use proxmox_notify::Config;
use proxmox_notify::context::pbs::PBS_CONTEXT;
use proxmox_notify::context::pve::PVE_CONTEXT;
use proxmox_sys::fs;

const PVE_CFG_PATH: &str = "/etc/pve";
const PVE_PUB_NOTIFICATION_CFG_FILENAME: &str = "/etc/pve/notifications.cfg";
const PVE_PRIV_NOTIFICATION_CFG_FILENAME: &str = "/etc/pve/priv/notifications.cfg";

const PBS_CFG_PATH: &str = "/etc/proxmox-backup";
const PBS_PUB_NOTIFICATION_CFG_FILENAME: &str = "/etc/proxmox-backup/notifications.cfg";
const PBS_PRIV_NOTIFICATION_CFG_FILENAME: &str = "/etc/proxmox-backup/notifications-priv.cfg";

/// Wrapper around `proxmox_sys::fs::file_read_optional_string` which also returns `None` upon error
/// after logging it.
fn attempt_file_read<P: AsRef<Path>>(path: P) -> Option<String> {
    match fs::file_read_optional_string(path.as_ref()) {
        Ok(contents) => contents,
        Err(err) => {
            error!("unable to read {path:?}: {err:#}", path = path.as_ref());
            None
        }
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

fn forward_common(mail: &[u8], config: &Config) -> Result<(), Error> {
    let real_uid = nix::unistd::getuid();
    // The uid is passed so that `sendmail` can be called as the a correct user.
    // (sendmail will show a warning if called from a setuid process)
    let notification =
        proxmox_notify::Notification::new_forwarded_mail(mail, Some(real_uid.as_raw()))?;

    proxmox_notify::api::common::send(config, &notification)?;

    Ok(())
}

/// Forward a mail to PVE's notification system
fn forward_for_pve(mail: &[u8]) -> Result<(), Error> {
    proxmox_notify::context::set_context(&PVE_CONTEXT);
    let config = attempt_file_read(PVE_PUB_NOTIFICATION_CFG_FILENAME).unwrap_or_default();
    let priv_config = attempt_file_read(PVE_PRIV_NOTIFICATION_CFG_FILENAME).unwrap_or_default();

    let config = Config::new(&config, &priv_config)?;

    forward_common(mail, &config)
}

/// Forward a mail to PBS's notification system
fn forward_for_pbs(mail: &[u8], has_pve: bool) -> Result<(), Error> {
    proxmox_notify::context::set_context(&PBS_CONTEXT);

    let config = if Path::new(PBS_PUB_NOTIFICATION_CFG_FILENAME).exists() {
        let config = attempt_file_read(PBS_PUB_NOTIFICATION_CFG_FILENAME).unwrap_or_default();
        let priv_config = attempt_file_read(PBS_PRIV_NOTIFICATION_CFG_FILENAME).unwrap_or_default();

        Config::new(&config, &priv_config)?
    } else {
        // Instantiate empty config.
        // Note: This will contain the default built-in targets/matchers.
        let config = Config::new("", "")?;
        if has_pve {
            // Skip forwarding if we are co-installed with PVE AND
            // we do not have our own notifications.cfg file yet
            // --> We assume that PVE has a sane matcher configured that
            // forwards the mail properly
            // TODO: This can be removed once PBS has full notification integration

            return Ok(());
        }
        config
    };

    forward_common(mail, &config)?;

    Ok(())
}

fn main() {
    if let Err(err) = Logger::from_env("PROXMOX_LOG", LevelFilter::INFO)
        .journald()
        .init()
    {
        eprintln!("unable to initialize syslog: {err}");
    }

    // Read the mail that is to be forwarded from stdin
    match read_stdin() {
        Ok(mail) => {
            let mut has_pve = false;

            // Assume a PVE installation if /etc/pve exists
            if Path::new(PVE_CFG_PATH).exists() {
                has_pve = true;
                if let Err(err) = forward_for_pve(&mail) {
                    error!("could not forward mail for Proxmox VE: {err:#}");
                }
            }

            // Assume a PBS installation if /etc/proxmox-backup exists
            if Path::new(PBS_CFG_PATH).exists() {
                if let Err(err) = forward_for_pbs(&mail, has_pve) {
                    error!("could not forward mail for Proxmox Backup Server: {err:#}");
                }
            }
        }
        Err(err) => {
            error!("could not read mail from STDIN: {err:#}")
        }
    }
}
