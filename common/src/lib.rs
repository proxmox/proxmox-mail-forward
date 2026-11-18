//! Definitions shared between the Proxmox mail forwarder and the notification worker.

#![deny(unsafe_code)]
#![deny(missing_docs)]

use std::path::Path;

use anyhow::{Context, Error};
use nix::unistd::{Gid, Group, Uid, User};

/// A Proxmox product whose notifications are processed via a notification queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Product {
    /// Proxmox VE
    Pve,
    /// Proxmox Backup Server
    Pbs,
}

impl Product {
    /// All supported products, in order of precedence.
    pub const ALL: &[Product] = &[Product::Pve, Product::Pbs];

    /// Directory of the product's notification queue.
    pub fn queue_dir(self) -> &'static str {
        match self {
            Self::Pve => "/var/lib/pve-manager/notifications/queue",
            Self::Pbs => "/var/lib/proxmox-backup/notifications/queue",
        }
    }

    /// Check whether the product is installed on this host.
    ///
    /// A product counts as installed if its notification queue directory exists.
    pub fn is_installed(self) -> bool {
        Path::new(self.queue_dir()).exists()
    }

    /// Name of the unprivileged user that notifications are submitted as.
    pub fn user(self) -> &'static str {
        match self {
            Self::Pve => "www-data",
            Self::Pbs => "backup",
        }
    }

    /// Name of the unprivileged group that notifications are submitted as.
    pub fn group(self) -> &'static str {
        match self {
            Self::Pve => "www-data",
            Self::Pbs => "backup",
        }
    }

    /// Look up the ID of the user that notifications are submitted as.
    pub fn uid(self) -> Result<Uid, Error> {
        let user = self.user();

        let uid = User::from_name(user)
            .with_context(|| format!("could not look up username {user}"))?
            .with_context(|| format!("user {user} does not exist"))?
            .uid;

        Ok(uid)
    }

    /// Look up the ID of the group that notifications are submitted as.
    pub fn gid(self) -> Result<Gid, Error> {
        let group = self.group();

        let gid = Group::from_name(group)
            .with_context(|| format!("could not look up group {group}"))?
            .with_context(|| format!("group {group} does not exist"))?
            .gid;

        Ok(gid)
    }
}
