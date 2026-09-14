use gpui::Subscription;
use std::{cell::Cell, rc::Rc};

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
