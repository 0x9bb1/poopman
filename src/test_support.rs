use gpui::{Subscription, TestAppContext, VisualTestContext};
use std::{cell::Cell, rc::Rc, time::Duration};

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

/// Let an open gpui-component dialog reach its resting position, then redraw.
///
/// The dialog slides 30px down over 0.25 s of wall-clock time (dialog.rs in
/// gpui-component 0.5.1), and test builds redraw the window after every
/// simulated event. A click aimed at bounds read mid-animation is hit-tested
/// against a later frame where the button has moved, so on a slow machine it
/// misses. Call this after the dialog has been drawn once (any `update`
/// draws it) and before reading bounds inside it. A finished animation stops
/// requesting frames, so the bounds then stay put.
pub(crate) fn finish_dialog_animation(cx: &mut VisualTestContext) {
    std::thread::sleep(Duration::from_millis(300));
    cx.update(|window, cx| window.draw(cx).clear());
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
