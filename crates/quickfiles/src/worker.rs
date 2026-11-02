//! Background worker thread. The UI thread never touches the (dummy) file system:
//! it sends a [`Command`], keeps rendering, and picks the [`Event`] up on a later frame.

use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
    time::Duration,
};

use eframe::egui;

use crate::{data, model::Listing};

pub enum Command {
    Load { id: u64, path: PathBuf },
}

pub enum Event {
    Loaded { id: u64, listing: Arc<Listing> },
}

pub struct Worker {
    tx: Option<mpsc::Sender<Command>>,
    rx: mpsc::Receiver<Event>,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (evt_tx, evt_rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("fs-worker".into())
            .spawn(move || run(&cmd_rx, &evt_tx, &ctx))
            .expect("failed to spawn worker thread");
        Self {
            tx: Some(cmd_tx),
            rx: evt_rx,
            handle: Some(handle),
        }
    }

    pub fn send(&self, command: Command) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(command);
        }
    }

    /// Non-blocking: drains everything the worker has finished so far.
    pub fn events(&self) -> impl Iterator<Item = Event> + '_ {
        self.rx.try_iter()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing the command channel makes `recv()` fail, which ends the thread loop.
        self.tx.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run(commands: &mpsc::Receiver<Command>, events: &mpsc::Sender<Event>, ctx: &egui::Context) {
    while let Ok(mut command) = commands.recv() {
        // Coalesce: if the user clicked around quickly, only the newest request matters.
        while let Ok(newer) = commands.try_recv() {
            command = newer;
        }
        let Command::Load { id, path } = command;

        thread::sleep(Duration::from_millis(120)); // pretend to hit the disk
        let listing = Arc::new(data::build_listing(&path));

        if events.send(Event::Loaded { id, listing }).is_err() {
            break; // UI is gone
        }
        ctx.request_repaint(); // wake the UI thread so it picks the result up
    }
}
