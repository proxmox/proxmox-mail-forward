//! This binary hosts the notification worker for PVE.
//!
//! When sending a notification, it is written as a JSON file into the queue directory at
//! `/var/lib/pve-manager/notifications/queue`.
//!
//! The notification worker will periodically read the contents of the queue directory and process
//! any notification that was put there.
//!
//! The worker task was implemented in such a way that it tries to finish sending the currently
//! handled notification in case of a requested daemon shutdown. If there are multiple
//! notifications in the queue, they are handled one-by-one, giving ample opportunities for
//! a clean shutdown.

#![deny(unsafe_code)]
#![deny(missing_docs)]

use std::time::Duration;

use anyhow::{Context, Error};
use tracing::level_filters::LevelFilter;

use proxmox_notify::context::pve::PVEContext;
use proxmox_notify::queue::{QueueConsumer, QueueWorker};

use common::Product;

fn main() -> Result<(), Error> {
    proxmox_log::Logger::from_env("PVE_LOG", LevelFilter::INFO)
        .journald()
        .init()?;

    proxmox_async::runtime::main(run())
}

async fn run() -> Result<(), Error> {
    let signal_task = proxmox_daemon::shutdown_signal_task().context("unable to start daemon")?;
    tokio::task::spawn(signal_task);

    tracing::info!("PVE notification worker starting up");

    start_notification_worker().await?;

    tracing::info!("PVE notification worker shutting down");

    Ok(())
}

/// Start the notification worker task
async fn start_notification_worker() -> Result<(), Error> {
    let consumer = QueueConsumer::new(Product::Pve.queue_dir());

    let notification_worker = std::pin::pin!(
        QueueWorker::builder(proxmox_notify::Context::new(PVEContext), consumer)
            .interval(Duration::from_secs(5))
            .retry_attempts(2)
            .build()
            .run()
    );

    let abort_future = std::pin::pin!(proxmox_daemon::shutdown_future());
    futures::future::select(notification_worker, abort_future).await;

    Ok(())
}
