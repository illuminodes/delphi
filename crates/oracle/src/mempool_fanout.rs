use std::collections::HashSet;
use std::net::SocketAddrV4;
use std::thread;
use std::time::Duration;

use mantle::hash::Hash256;
use mantle::network::Network;

use crate::mempool_watch::{MempoolWatcher, SeenTransaction};

const CONNECT_ATTEMPTS: usize = 48;
const SEEN_LIMIT: usize = 500_000;

/// What one peer contributed, from the fanout's point of view.
pub struct PeerStat {
    pub address: SocketAddrV4,
    pub announced: u64,
    pub first_sightings: u64,
    pub delivered: u64,
    pub live: bool,
}

impl PeerStat {
    /// Share of this peer's announcements that no other peer had sent yet.
    ///
    /// A peer whose announcements are all duplicates adds latency headroom
    /// but no new transactions.
    #[must_use]
    pub fn novelty(&self) -> f64 {
        if self.announced == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let share = self.first_sightings as f64 / self.announced as f64;
        share
    }

    /// Share of requested bodies this peer actually sent.
    ///
    /// A peer that announces freely but delivers nothing scores zero here,
    /// which is what a rejected `getdata` looks like from outside.
    #[must_use]
    pub fn delivery_rate(&self) -> f64 {
        if self.first_sightings == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let share = self.delivered as f64 / self.first_sightings as f64;
        share
    }
}

struct PeerLink {
    address: SocketAddrV4,
    watcher: MempoolWatcher,
    announced: u64,
    first_sightings: u64,
    live: bool,
}

impl PeerLink {
    fn stat(&self) -> PeerStat {
        PeerStat {
            address: self.address,
            announced: self.announced,
            first_sightings: self.first_sightings,
            delivered: self.watcher.delivered(),
            live: self.live,
        }
    }
}

/// Several [`MempoolWatcher`]s polled as one stream, deduplicated by txid.
///
/// Each announced transaction is requested from the first peer that offered
/// it, so adding peers widens coverage without multiplying bandwidth. The
/// per-peer first-sighting counts say how much each peer actually added.
pub struct PeerFanout {
    links: Vec<PeerLink>,
    seen: HashSet<Hash256>,
    duplicate_announcements: u64,
}

impl PeerFanout {
    /// Connect to up to `want` peers, attempting candidates in parallel.
    ///
    /// Peers that time out or fail the handshake are skipped; the result may
    /// hold fewer than `want` links, or none.
    #[must_use]
    pub fn connect(
        candidates: &[SocketAddrV4],
        network: Network,
        want: usize,
        timeout: Duration,
        poll_timeout: Duration,
    ) -> Self {
        let attempts: Vec<SocketAddrV4> =
            candidates.iter().copied().take(CONNECT_ATTEMPTS).collect();
        let mut links = Vec::new();
        for wave in attempts.chunks(want.max(1)) {
            let connected = Self::connect_wave(wave, network, timeout, poll_timeout);
            links.extend(connected);
            if links.len() >= want {
                links.truncate(want);
                break;
            }
        }
        Self {
            links,
            seen: HashSet::new(),
            duplicate_announcements: 0,
        }
    }

    fn connect_wave(
        wave: &[SocketAddrV4],
        network: Network,
        timeout: Duration,
        poll_timeout: Duration,
    ) -> Vec<PeerLink> {
        thread::scope(|scope| {
            let handles: Vec<_> = wave
                .iter()
                .map(|address| {
                    let address = *address;
                    scope.spawn(move || {
                        let mut watcher = MempoolWatcher::connect(address, network, timeout).ok()?;
                        watcher.set_poll_timeout(poll_timeout).ok()?;
                        Some(PeerLink {
                            address,
                            watcher,
                            announced: 0,
                            first_sightings: 0,
                            live: true,
                        })
                    })
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|handle| handle.join().ok().flatten())
                .collect()
        })
    }

    /// Poll every live peer once, returning the transactions that arrived.
    ///
    /// Announcements already seen on another peer are counted and dropped
    /// rather than re-requested. Peers that disconnect are marked dead and
    /// stop being polled, but keep their statistics.
    pub fn poll(&mut self) -> Vec<SeenTransaction> {
        if self.seen.len() > SEEN_LIMIT {
            self.seen.clear();
        }
        let mut delivered = Vec::new();
        for link in &mut self.links {
            if !link.live {
                continue;
            }
            let Ok(batch) = link.watcher.poll() else {
                link.live = false;
                continue;
            };
            link.announced += batch.announced.len() as u64;
            let mut wanted = Vec::new();
            for hash in batch.announced {
                if self.seen.insert(hash) {
                    link.first_sightings += 1;
                    wanted.push(hash);
                } else {
                    self.duplicate_announcements += 1;
                }
            }
            if !wanted.is_empty() && link.watcher.request(&wanted).is_err() {
                link.live = false;
            }
            delivered.extend(batch.transactions);
        }
        delivered
    }

    /// Distinct transactions announced by any peer — the coverage ceiling.
    #[must_use]
    pub fn unique_announced(&self) -> usize {
        self.seen.len()
    }

    #[must_use]
    pub const fn duplicate_announcements(&self) -> u64 {
        self.duplicate_announcements
    }

    #[must_use]
    pub fn live_peers(&self) -> usize {
        self.links.iter().filter(|link| link.live).count()
    }

    #[must_use]
    pub fn peer_count(&self) -> usize {
        self.links.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// Per-peer contribution, best first sightings first.
    #[must_use]
    pub fn stats(&self) -> Vec<PeerStat> {
        let mut stats: Vec<PeerStat> = self.links.iter().map(PeerLink::stat).collect();
        stats.sort_by_key(|stat| std::cmp::Reverse(stat.first_sightings));
        stats
    }

    /// Best single peer's announcement count, for comparison against the
    /// deduplicated total.
    #[must_use]
    pub fn best_single_peer(&self) -> u64 {
        self.links
            .iter()
            .map(|link| link.announced)
            .max()
            .unwrap_or(0)
    }

    /// Bodies refused across all peers, and `tx` frames that would not parse.
    ///
    /// These explain any gap between announcements and delivered bodies.
    #[must_use]
    pub fn delivery_losses(&self) -> (u64, u64) {
        self.links.iter().fold((0, 0), |(refused, bad), link| {
            (
                refused + link.watcher.not_found(),
                bad + link.watcher.decode_failures(),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn novelty_is_zero_for_a_silent_peer() {
        let stat = PeerStat {
            address: "127.0.0.1:8333".parse().unwrap(),
            announced: 0,
            first_sightings: 0,
            delivered: 0,
            live: true,
        };
        assert!((stat.novelty() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn novelty_reports_the_share_that_arrived_first() {
        let stat = PeerStat {
            address: "127.0.0.1:8333".parse().unwrap(),
            announced: 200,
            first_sightings: 50,
            delivered: 50,
            live: true,
        };
        assert!((stat.novelty() - 0.25).abs() < 1e-12);
    }

    #[test]
    fn a_peer_that_announces_but_never_delivers_scores_zero() {
        let stat = PeerStat {
            address: "127.0.0.1:8333".parse().unwrap(),
            announced: 9_000,
            first_sightings: 1_100,
            delivered: 0,
            live: true,
        };
        assert!((stat.delivery_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn delivery_rate_reports_the_share_of_bodies_returned() {
        let stat = PeerStat {
            address: "127.0.0.1:8333".parse().unwrap(),
            announced: 400,
            first_sightings: 200,
            delivered: 150,
            live: true,
        };
        assert!((stat.delivery_rate() - 0.75).abs() < 1e-12);
    }

    #[test]
    fn a_fanout_with_no_peers_is_empty() {
        let fanout = PeerFanout {
            links: Vec::new(),
            seen: HashSet::new(),
            duplicate_announcements: 0,
        };
        assert!(fanout.is_empty());
        assert_eq!(fanout.live_peers(), 0);
        assert_eq!(fanout.best_single_peer(), 0);
        assert_eq!(fanout.unique_announced(), 0);
    }
}
