use std::path::Path;
use std::process::Command;

use anyhow::{bail, format_err, Error};
use serde::Deserialize;

use proxmox_schema::{ObjectSchema, Schema, StringSchema};
use proxmox_section_config::{SectionConfig, SectionConfigPlugin};
use proxmox_sys::fs;

const PBS_USER_CFG_FILENAME: &str = "/etc/proxmox-backup/user.cfg";
const PBS_ROOT_USER: &str = "root@pam";

// FIXME: Switch to the actual schema when possible in terms of dependency.
// It's safe to assume that the config was written with the actual schema restrictions, so parsing
// it with the less restrictive schema should be enough for the purpose of getting the mail address.
const DUMMY_ID_SCHEMA: Schema = StringSchema::new("dummy ID").min_length(3).schema();
const DUMMY_EMAIL_SCHEMA: Schema = StringSchema::new("dummy email").schema();
const DUMMY_USER_SCHEMA: ObjectSchema = ObjectSchema {
    description: "minimal PBS user",
    properties: &[
        ("userid", false, &DUMMY_ID_SCHEMA),
        ("email", true, &DUMMY_EMAIL_SCHEMA),
    ],
    additional_properties: true,
    default_key: None,
};

#[derive(Deserialize)]
struct DummyPbsUser {
    pub email: Option<String>,
}

const PVE_USER_CFG_FILENAME: &str = "/etc/pve/user.cfg";
const PVE_DATACENTER_CFG_FILENAME: &str = "/etc/pve/datacenter.cfg";
const PVE_ROOT_USER: &str = "root@pam";

/// Convenience helper to get the trimmed contents of an optional &str, mapping blank ones to `None`
/// and creating a String from it for returning.
fn normalize_for_return(s: Option<&str>) -> Option<String> {
    match s?.trim() {
        "" => None,
        s => Some(s.to_string()),
    }
}

/// Extract the root user's email address from the PBS user config.
fn get_pbs_mail_to(content: &str) -> Option<String> {
    let mut config = SectionConfig::new(&DUMMY_ID_SCHEMA).allow_unknown_sections(true);
    let user_plugin = SectionConfigPlugin::new(
        "user".to_string(),
        Some("userid".to_string()),
        &DUMMY_USER_SCHEMA,
    );
    config.register_plugin(user_plugin);

    match config.parse(PBS_USER_CFG_FILENAME, content) {
        Ok(parsed) => {
            parsed.sections.get(PBS_ROOT_USER)?;
            match parsed.lookup::<DummyPbsUser>("user", PBS_ROOT_USER) {
                Ok(user) => normalize_for_return(user.email.as_deref()),
                Err(err) => {
                    log::error!("unable to parse {} - {}", PBS_USER_CFG_FILENAME, err);
                    None
                }
            }
        }
        Err(err) => {
            log::error!("unable to parse {} - {}", PBS_USER_CFG_FILENAME, err);
            None
        }
    }
}

/// Extract the root user's email address from the PVE user config.
fn get_pve_mail_to(content: &str) -> Option<String> {
    normalize_for_return(content.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        #[allow(clippy::get_first)] // to keep expression style consistent
        match fields.get(0)?.trim() == "user" && fields.get(1)?.trim() == PVE_ROOT_USER {
            true => fields.get(6).copied(),
            false => None,
        }
    }))
}

/// Extract the From-address configured in the PVE datacenter config.
fn get_pve_mail_from(content: &str) -> Option<String> {
    normalize_for_return(
        content
            .lines()
            .find_map(|line| line.strip_prefix("email_from:")),
    )
}

/// Executes sendmail as a child process with the specified From/To-addresses, expecting the mail
/// contents to be passed via stdin inherited from this program.
fn forward_mail(mail_from: String, mail_to: Vec<String>) -> Result<(), Error> {
    if mail_to.is_empty() {
        bail!("user 'root@pam' does not have an email address");
    }

    log::info!("forward mail to <{}>", mail_to.join(","));

    let mut cmd = Command::new("sendmail");
    cmd.args([
        "-bm", "-N", "never", // never send DSN (avoid mail loops)
        "-f", &mail_from, "--",
    ]);
    cmd.args(mail_to);
    cmd.env("PATH", "/sbin:/bin:/usr/sbin:/usr/bin");

    // with status(), child inherits stdin
    cmd.status()
        .map_err(|err| format_err!("command {:?} failed - {}", cmd, err))?;

    Ok(())
}

/// Wrapper around `proxmox_sys::fs::file_read_optional_string` which also returns `None` upon error
/// after logging it.
fn attempt_file_read<P: AsRef<Path>>(path: P) -> Option<String> {
    match fs::file_read_optional_string(path) {
        Ok(contents) => contents,
        Err(err) => {
            log::error!("{}", err);
            None
        }
    }
}

fn main() {
    if let Err(err) = syslog::init(
        syslog::Facility::LOG_DAEMON,
        log::LevelFilter::Info,
        Some("proxmox-mail-forward"),
    ) {
        eprintln!("unable to inititialize syslog - {}", err);
    }

    let pbs_user_cfg_content = attempt_file_read(PBS_USER_CFG_FILENAME);
    let pve_user_cfg_content = attempt_file_read(PVE_USER_CFG_FILENAME);
    let pve_datacenter_cfg_content = attempt_file_read(PVE_DATACENTER_CFG_FILENAME);

    let real_uid = nix::unistd::getuid();
    if let Err(err) = nix::unistd::setresuid(real_uid, real_uid, real_uid) {
        log::error!(
            "mail forward failed: unable to set effective uid to {}: {}",
            real_uid,
            err
        );
        return;
    }

    let pbs_mail_to = pbs_user_cfg_content.and_then(|content| get_pbs_mail_to(&content));
    let pve_mail_to = pve_user_cfg_content.and_then(|content| get_pve_mail_to(&content));
    let pve_mail_from = pve_datacenter_cfg_content.and_then(|content| get_pve_mail_from(&content));

    let mail_from = pve_mail_from.unwrap_or_else(|| "root".to_string());

    let mut mail_to = vec![];
    if let Some(pve_mail_to) = pve_mail_to {
        mail_to.push(pve_mail_to);
    }
    if let Some(pbs_mail_to) = pbs_mail_to {
        if !mail_to.contains(&pbs_mail_to) {
            mail_to.push(pbs_mail_to);
        }
    }

    if let Err(err) = forward_mail(mail_from, mail_to) {
        log::error!("mail forward failed: {}", err);
    }
}
