use std::cell::RefCell;
use std::rc::Rc;

use nostr_minions::{use_nostr_relay_pool, NostrRelayPoolStore, NostrSubscription, SubscriptionId, WallClock};
use yew::prelude::*;

use crate::{DashState, FeedFilter, HistoryRange, NoteGate, PublisherKey};

/// The publisher's notes, gathered from two sets of subscriptions: the live and transaction
/// streams, opened once per publisher, and the audited history, reopened whenever the range changes.
#[hook]
pub fn use_dash_state(publisher: &PublisherKey, range: HistoryRange) -> DashState {
    let pool = use_nostr_relay_pool();
    let state = use_mut_ref(DashState::default);
    let refresh = use_force_update();
    let streams: Rc<RefCell<Vec<SubscriptionId>>> = use_mut_ref(Vec::new);
    let history: Rc<RefCell<Vec<SubscriptionId>>> = use_mut_ref(Vec::new);

    let Some(pool) = pool else {
        return DashState::default();
    };
    let callback = Callback::from({
        let state = state.clone();
        let gate = NoteGate::new(publisher.hex());
        move |note| {
            if !gate.admits(&note) {
                return;
            }
            state.borrow_mut().accept(&note);
            refresh.force_update();
        }
    });

    use_effect_with(publisher.hex().to_string(), {
        let state = state.clone();
        let pool = pool.clone();
        let callback = callback.clone();
        move |publisher_hex| {
            *state.borrow_mut() = DashState::default();
            let feed = FeedFilter::new(publisher_hex.clone());
            let subscriptions = DashSubscriptions::new(pool, streams);
            subscriptions.open(vec![feed.live(), feed.transactions()], &callback);
            move || subscriptions.close()
        }
    });

    use_effect_with((publisher.hex().to_string(), range), {
        let state = state.clone();
        move |(publisher_hex, range)| {
            let now = WallClock::unix_seconds();
            state.borrow_mut().restart_history(*range, now);
            let feed = FeedFilter::new(publisher_hex.clone());
            let subscriptions = DashSubscriptions::new(pool, history);
            subscriptions.open(feed.audits(now, *range), &callback);
            move || subscriptions.close()
        }
    });

    let snapshot = state.borrow().clone();
    snapshot
}

/// One group of relay subscriptions that open and close together.
struct DashSubscriptions {
    pool: NostrRelayPoolStore,
    ids: Rc<RefCell<Vec<SubscriptionId>>>,
}

impl DashSubscriptions {
    const fn new(pool: NostrRelayPoolStore, ids: Rc<RefCell<Vec<SubscriptionId>>>) -> Self {
        Self { pool, ids }
    }

    fn open(&self, filters: Vec<NostrSubscription>, callback: &Callback<nostr_minions::NostrNote>) {
        let opened: Vec<SubscriptionId> = filters
            .into_iter()
            .map(|filter| self.pool.subscribe(filter, callback.clone()))
            .collect();
        *self.ids.borrow_mut() = opened;
    }

    fn close(&self) {
        for id in self.ids.borrow_mut().drain(..) {
            self.pool.unsubscribe(&id);
        }
    }
}
