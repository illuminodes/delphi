use gloo_timers::callback::Interval;
use nostr_minions::WallClock;
use yew::prelude::*;

#[hook]
pub fn use_unix_seconds(period_millis: u32) -> i64 {
    let now = use_state(WallClock::unix_seconds);
    use_effect_with(period_millis, {
        let now = now.clone();
        move |period| {
            let interval = Interval::new(*period, move || now.set(WallClock::unix_seconds()));
            move || drop(interval)
        }
    });
    *now
}
