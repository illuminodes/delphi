use std::net::{SocketAddr, SocketAddrV4, ToSocketAddrs};
use std::path::{Path, PathBuf};

use mantle::network::Network;

#[derive(Debug, Clone, Default)]
pub struct PeerSources {
    explicit: Vec<SocketAddrV4>,
    files: Vec<PathBuf>,
}

impl PeerSources {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_explicit(&mut self, peer: SocketAddrV4) {
        self.explicit.push(peer);
    }

    pub fn add_file(&mut self, path: impl Into<PathBuf>) {
        self.files.push(path.into());
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.explicit.is_empty() && self.files.is_empty()
    }

    #[must_use]
    pub fn resolve(&self, network: Network) -> Vec<SocketAddrV4> {
        let mut peers = self.explicit.clone();
        self.load_files(&mut peers);
        for seed in network.default_seeds() {
            if let Ok(address) = seed.parse() {
                peers.push(address);
                continue;
            }
            match seed.to_socket_addrs() {
                Ok(addresses) => peers.extend(addresses.filter_map(|address| match address {
                    SocketAddr::V4(address) => Some(address),
                    SocketAddr::V6(_) => None,
                })),
                Err(error) => eprintln!("warning: failed to resolve seed {seed}: {error}"),
            }
        }
        peers.sort_unstable();
        peers.dedup();
        peers
    }

    fn load_files(&self, peers: &mut Vec<SocketAddrV4>) {
        for path in &self.files {
            if let Err(error) = Self::append_file(path, peers) {
                eprintln!("warning: {error}");
            }
        }
    }

    fn append_file(path: &Path, peers: &mut Vec<SocketAddrV4>) -> Result<(), String> {
        let body = std::fs::read_to_string(path)
            .map_err(|error| format!("read peer file {}: {error}", path.display()))?;
        for (line_index, line) in body.lines().enumerate() {
            let value = line.split('#').next().unwrap_or("").trim();
            if value.is_empty() {
                continue;
            }
            peers.push(value.parse().map_err(|error| {
                format!("{}:{}: invalid peer {value}: {error}", path.display(), line_index + 1)
            })?);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::SocketAddrV4;

    use mantle::network::Network;

    use super::PeerSources;

    struct TempPeerFile;

    impl TempPeerFile {
        fn write(body: &str) -> std::path::PathBuf {
            let path = std::env::temp_dir().join(format!("mantle-peers-{}.txt", std::process::id()));
            let mut file = std::fs::File::create(&path).expect("create temp peer file");
            file.write_all(body.as_bytes()).expect("write temp peer file");
            path
        }
    }

    #[test]
    fn merges_explicit_and_file_peers_deduplicated() {
        let mut sources = PeerSources::new();
        sources.add_explicit("127.0.0.1:8333".parse::<SocketAddrV4>().expect("valid"));
        sources.add_explicit("10.0.0.1:8333".parse::<SocketAddrV4>().expect("valid"));
        let path = TempPeerFile::write("127.0.0.1:8333\n# comment\n\n9.9.9.9:8333\n");
        sources.add_file(&path);

        let peers = sources.resolve(Network::Regtest);
        let _ = std::fs::remove_file(&path);

        let addresses: Vec<String> = peers.iter().map(ToString::to_string).collect();
        assert_eq!(addresses, ["9.9.9.9:8333", "10.0.0.1:8333", "127.0.0.1:8333"]);
    }

    #[test]
    fn regtest_has_no_seeds_so_only_explicit_peers_remain() {
        let mut sources = PeerSources::new();
        sources.add_explicit("127.0.0.1:18444".parse::<SocketAddrV4>().expect("valid"));
        let peers = sources.resolve(Network::Regtest);
        assert_eq!(peers.len(), 1);
    }

    #[test]
    fn reports_emptiness() {
        assert!(PeerSources::new().is_empty());
        let mut sources = PeerSources::new();
        sources.add_explicit("127.0.0.1:18444".parse::<SocketAddrV4>().expect("valid"));
        assert!(!sources.is_empty());
    }
}
