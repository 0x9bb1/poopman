use gpui::{Subscription, TestAppContext};
use std::{cell::Cell, rc::Rc};

use crate::db::Database;

/// Run every task to completion, including tasks awaiting database replies.
///
/// Replies come from the database thread, which gpui's test scheduler cannot
/// see, so `run_until_parked` alone returns while a task still waits on one.
/// The database runs jobs in order and each reply wakes its task before the
/// next job starts: once `flush` returns, every earlier reply has made its
/// task runnable. Repeat until a round wakes nothing.
pub(crate) fn settle(cx: &mut TestAppContext, db: &Database) {
    loop {
        cx.run_until_parked();
        db.flush().wait().expect("database thread is running");
        if !cx.executor().tick() {
            return;
        }
    }
}

/// Observe disposal of real GPUI listeners without depending on GPUI internals.
/// Track application-owned subscriptions, not total InputState lifetime: the
/// pinned gpui-component 0.5.1 has an InputState/MouseContextMenu strong cycle.
#[derive(Default)]
pub(crate) struct SubscriptionTracker(Rc<Cell<usize>>);

impl SubscriptionTracker {
    pub(crate) fn track(&self, subscriptions: &mut [Subscription]) {
        for subscription in subscriptions {
            self.0.set(self.0.get() + 1);
            let count = self.0.clone();
            let original = std::mem::replace(subscription, Subscription::new(|| {}));
            *subscription = Subscription::join(
                original,
                Subscription::new(move || {
                    count.set(count.get() - 1);
                }),
            );
        }
    }

    pub(crate) fn live(&self) -> usize {
        self.0.get()
    }
}
