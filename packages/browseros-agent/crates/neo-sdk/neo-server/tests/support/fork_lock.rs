//! One lock per test process, held while a port is probed and while a child is spawned.
//!
//! On macOS a socket is made close-on-exec only after `socket()` returns. A child that another
//! test's thread spawns in between inherits it. A port probe then outlives its own drop, still
//! listening inside that child:
//! - the binary given the port cannot bind it;
//! - or a test's request lands in a queue nobody accepts from, and is reset when that child exits.
//!
//! Measured in `process_contract`, with its tests in parallel: 14 resets in 1,200 runs, and none
//! with one test thread. Holding this lock for both keeps every probe out of every child.
//!
//! Each test file that spawns includes it as a module, and so does `browseros_rig.rs`.

use std::error::Error;
use std::io;
use std::net::{Ipv4Addr, TcpListener};
use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::process::{Child, Command};

/// What a probe and a spawn take turns under.
static FORKING: Mutex<()> = Mutex::new(());

fn held() -> MutexGuard<'static, ()> {
    FORKING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A port nothing listens on right now. The probe is closed before any child of this process
/// can be spawned. Another process could still take the port before its user binds; that race
/// is accepted for a test.
pub(crate) fn free_port() -> Result<u16, Box<dyn Error>> {
    let _probing = held();
    Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?
        .local_addr()?
        .port())
}

/// `command` spawned while no port is being probed. Every child a test starts goes through
/// here, `output()` and `status()` included, since both spawn too.
pub(crate) fn spawn(command: &mut Command) -> io::Result<Child> {
    let _forking = held();
    command.spawn()
}
