// SPDX-License-Identifier: MPL-2.0

use iced::Task;
use std::time::{Duration, Instant};

const TOAST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ToastId(u64);

#[derive(Default)]
pub struct Toasts {
    next_id: u64,
    items: Vec<(ToastId, String)>,
}

impl Toasts {
    /// Adds a toast and returns a task that yields its id once it should auto-close.
    pub fn push<
        Message: std::marker::Send + 'static,
        M: FnMut(ToastId) -> Message + std::marker::Send + 'static,
    >(
        &mut self,
        toast_message: impl Into<String>,
        close_message: M,
    ) -> Task<Message> {
        let id = ToastId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.items.push((id, toast_message.into()));
        Task::perform(smol::Timer::after(TOAST_TIMEOUT), move |_| id).map(close_message)
    }

    /// Removes the toast; a no-op if it was already closed.
    pub fn close(&mut self, id: ToastId) {
        self.items.retain(|(toast_id, _)| *toast_id != id);
    }

    pub fn iter(&self) -> impl Iterator<Item = &(ToastId, String)> {
        self.items.iter()
    }
}
