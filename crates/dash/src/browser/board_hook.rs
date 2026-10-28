use std::cell::RefCell;
use std::rc::Rc;

use gloo_timers::callback::Timeout;
use nostr_minions::{
    use_nostr_relay_pool, NostrNote, NostrRelayEvent, NostrRelayPoolStore, NostrSubscription, SubscriptionId, WallClock,
};
use yew::prelude::*;

use super::boot::Boot;
use super::history_store::HistoryStore;
use super::kept_store::KeptStore;
use super::render_pacer::RenderPacer;
use crate::{DashState, FeedFilter, HistoryCache, HistoryLoad, HistoryRange, NoteGate, PublisherKey};

/// The publisher's notes, gathered from two sets of subscriptions: the live, stored and
/// transaction streams, opened once per publisher, and the audited history, reopened whenever the
/// range changes. The history saved by earlier visits is drawn at once and the relay is asked only
/// for what came after it; what the relay sends is held back until all of it has arrived, and
/// redraws are paced, so the charts are drawn once rather than once per note.
#[hook]
pub fn use_dash_state(boot: &Boot, publisher: &PublisherKey, range: HistoryRange) -> DashState {
    let pool = use_nostr_relay_pool();
    let state = use_mut_ref(DashState::default);
    let load = use_mut_ref(HistoryLoad::default);
    let gate = NoteGate::new(publisher.hex());
    let cache = {
        let saved = boot.saved.clone();
        let gate = gate.clone();
        use_mut_ref(move || {
            let mut cache = HistoryCache::default();
            for note in saved.history.iter().filter(|note| gate.is_publisher(note)) {
                cache.keep(note);
            }
            cache
        })
    };
    let pacer = RenderPacer::new(use_force_update());
    let pacer = use_mut_ref(move || pacer).borrow().clone();
    let streams: Rc<RefCell<Vec<SubscriptionId>>> = use_mut_ref(Vec::new);
    let history: Rc<RefCell<Vec<SubscriptionId>>> = use_mut_ref(Vec::new);

    let Some(pool) = pool else {
        return DashState::default();
    };
    let feed = DashFeed {
        state: state.clone(),
        load,
        cache,
        pacer,
        gate,
        stores: boot.db.as_ref().map(|db| Rc::new((KeptStore::new(db, publisher.hex()), HistoryStore::new(db, publisher.hex())))),
    };

    use_effect_with(publisher.hex().to_string(), {
        let pool = pool.clone();
        let feed = feed.clone();
        let saved = boot.saved.clone();
        move |publisher_hex| {
            feed.restore(&saved.kept);
            let filter = FeedFilter::new(publisher_hex.clone());
            let subscriptions = DashSubscriptions::new(pool.clone(), streams);
            subscriptions.open(vec![filter.stored(), filter.live(), filter.transactions()], &feed.live_callback());
            let events = pool.subscribe_relay_events(feed.event_callback());
            move || {
                subscriptions.close();
                pool.unsubscribe_relay_events(&events);
            }
        }
    });

    use_effect_with((publisher.hex().to_string(), range), {
        move |(publisher_hex, range)| {
            let now = WallClock::unix_seconds();
            feed.restart_history(*range, now);
            let range_since = range.since(now);
            let filters = {
                let cache = feed.cache.borrow();
                FeedFilter::new(publisher_hex.clone()).audits(*range, |window| cache.since(window, range_since))
            };
            feed.load.borrow_mut().start();
            let subscriptions = DashSubscriptions::new(pool, history);
            let ids = subscriptions.open(filters, &feed.history_callback());
            feed.load.borrow_mut().wait_for(ids.iter().map(|id| id.as_str().to_string()));
            feed.show_loading();
            let fallback = {
                let feed = feed.clone();
                Timeout::new(DashFeed::HISTORY_WAIT_MILLIS, move || feed.show_history(|load| load.release()))
            };
            move || {
                fallback.cancel();
                subscriptions.close();
            }
        }
    });

    let snapshot = state.borrow().clone();
    snapshot
}

/// Where admitted notes go: the dashboard state, the held-back history, the history kept in the
/// browser, the redraw pacer and the browser's saved copies.
#[derive(Clone)]
struct DashFeed {
    state: Rc<RefCell<DashState>>,
    load: Rc<RefCell<HistoryLoad>>,
    cache: Rc<RefCell<HistoryCache>>,
    pacer: RenderPacer,
    gate: NoteGate,
    stores: Option<Rc<(KeptStore, HistoryStore)>>,
}

impl DashFeed {
    /// How long to wait for a relay to report the end of the stored history before showing what came.
    const HISTORY_WAIT_MILLIS: u32 = 8_000;

    /// Shows the repeated notes saved by an earlier visit, those that still pass the gate.
    fn restore(&self, kept: &[NostrNote]) {
        {
            let mut state = self.state.borrow_mut();
            *state = DashState::default();
            for note in kept.iter().filter(|note| self.gate.admits(note)) {
                state.restore(note);
            }
        }
        self.pacer.now();
    }

    /// Starts the history over for `range` and draws at once what the browser already keeps.
    fn restart_history(&self, range: HistoryRange, now: i64) {
        {
            let mut state = self.state.borrow_mut();
            state.restart_history(range, now);
            for note in self.cache.borrow().notes() {
                state.restore(note);
            }
        }
        self.pacer.now();
    }

    fn live_callback(&self) -> Callback<NostrNote> {
        let feed = self.clone();
        Callback::from(move |note: NostrNote| {
            if !feed.gate.admits(&note) {
                feed.state.borrow_mut().refuse();
                feed.pacer.soon();
                return;
            }
            let revision = feed.state.borrow().kept().revision();
            feed.state.borrow_mut().accept(&note);
            let state = feed.state.borrow();
            if let Some(stores) = feed.stores.as_ref().filter(|_| state.kept().revision() != revision) {
                stores.0.save(state.kept());
            }
            feed.pacer.soon();
        })
    }

    fn history_callback(&self) -> Callback<NostrNote> {
        let feed = self.clone();
        Callback::from(move |note: NostrNote| {
            if !feed.gate.admits(&note) {
                feed.state.borrow_mut().refuse();
            } else if let Some(note) = feed.load.borrow_mut().hold(note) {
                feed.keep_history(std::slice::from_ref(&note));
                feed.state.borrow_mut().accept(&note);
            }
            feed.show_loading();
            feed.pacer.soon();
        })
    }

    /// Adds admitted audit notes to the browser's copy, saving the new ones and forgetting the
    /// ones pushed out.
    fn keep_history(&self, notes: &[NostrNote]) {
        let mut added = Vec::new();
        let mut dropped = Vec::new();
        {
            let mut cache = self.cache.borrow_mut();
            for note in notes {
                if let Some(pushed_out) = cache.keep(note) {
                    added.push(note.clone());
                    dropped.extend(pushed_out);
                }
            }
        }
        if let Some(stores) = self.stores.as_ref().filter(|_| !added.is_empty() || !dropped.is_empty()) {
            stores.1.save(&added, dropped);
        }
    }

    fn event_callback(&self) -> Callback<NostrRelayEvent> {
        let feed = self.clone();
        Callback::from(move |event: NostrRelayEvent| {
            if let NostrRelayEvent::EndOfSubscription(_, id) = event {
                feed.show_history(|load| load.finish(&id));
            }
        })
    }

    /// Takes what `release` hands back from the held history and, once loading is over, draws it.
    fn show_history(&self, release: impl FnOnce(&mut HistoryLoad) -> Vec<NostrNote>) {
        let (notes, loading) = {
            let mut load = self.load.borrow_mut();
            if !load.is_loading() {
                return;
            }
            (release(&mut load), load.is_loading())
        };
        if loading {
            return;
        }
        self.keep_history(&notes);
        {
            let mut state = self.state.borrow_mut();
            for note in &notes {
                state.accept(note);
            }
        }
        self.show_loading();
        self.pacer.now();
    }

    fn show_loading(&self) {
        let load = self.load.borrow();
        self.state
            .borrow_mut()
            .set_history_loading(load.is_loading().then(|| load.held()));
    }
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

    fn open(&self, filters: Vec<NostrSubscription>, callback: &Callback<NostrNote>) -> Vec<SubscriptionId> {
        let opened: Vec<SubscriptionId> = filters
            .into_iter()
            .map(|filter| self.pool.subscribe(filter, callback.clone()))
            .collect();
        self.ids.borrow_mut().clone_from(&opened);
        opened
    }

    fn close(&self) {
        for id in self.ids.borrow_mut().drain(..) {
            self.pool.unsubscribe(&id);
        }
    }
}
