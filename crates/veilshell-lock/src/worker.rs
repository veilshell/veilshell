//! Serialized background PAM authentication worker thread.

use std::sync::mpsc;
use std::thread;

use smithay_client_toolkit::reexports::calloop::channel::Sender as LoopSender;
use veilshell_lock_input::attempt::Ticket;

use crate::pam::{authenticate, AuthOutcome};
use crate::secret::SecretPassword;

/// Result posted back to the Wayland event loop when PAM finishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthResult {
    pub ticket: Ticket,
    pub outcome: AuthOutcome,
}

struct AuthRequest {
    ticket: Ticket,
    password: SecretPassword,
}

/// Handle to the single background PAM authentication worker.
pub struct AuthWorker {
    tx: mpsc::Sender<AuthRequest>,
}

impl AuthWorker {
    /// Spawn a dedicated worker thread bound to one PAM service and account.
    #[must_use]
    pub fn spawn(service: String, username: String, reply: LoopSender<AuthResult>) -> Self {
        let (tx, rx) = mpsc::channel::<AuthRequest>();
        let _ = thread::Builder::new()
            .name("veilshell-lock-pam".to_owned())
            .spawn(move || {
                while let Ok(mut req) = rx.recv() {
                    let outcome = authenticate(&service, &username, &mut req.password);
                    if reply
                        .send(AuthResult {
                            ticket: req.ticket,
                            outcome,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        Self { tx }
    }

    /// Submit a password attempt for `ticket`.
    ///
    /// If sending fails, `password` is dropped and zeroized immediately.
    pub fn submit(&self, ticket: Ticket, password: SecretPassword) -> Result<(), ()> {
        self.tx
            .send(AuthRequest { ticket, password })
            .map_err(|_| ())
    }
}
