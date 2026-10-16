use crate::{PanelId, TileSize};

/// The smallest size, title strip and tabs included, at which a card's content stays readable.
pub struct PanelFootprint;

impl PanelFootprint {
    const INFO: TileSize = TileSize::new(300.0, 150.0);
    const PRICE: TileSize = TileSize::new(240.0, 150.0);
    const FIT: TileSize = TileSize::new(280.0, 220.0);
    const HISTORY: TileSize = TileSize::new(320.0, 230.0);
    const BLOCKCHAIN: TileSize = TileSize::new(360.0, 260.0);
    const MEMPOOL: TileSize = TileSize::new(340.0, 240.0);

    #[must_use]
    pub const fn minimum(panel: PanelId) -> TileSize {
        match panel {
            PanelId::Info => Self::INFO,
            PanelId::Price => Self::PRICE,
            PanelId::Fit => Self::FIT,
            PanelId::History => Self::HISTORY,
            PanelId::Blockchain => Self::BLOCKCHAIN,
            PanelId::Mempool => Self::MEMPOOL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PanelFootprint;
    use crate::PanelId;

    #[test]
    fn charts_need_more_room_than_prices() {
        let price = PanelFootprint::minimum(PanelId::Price);
        let history = PanelFootprint::minimum(PanelId::History);

        assert!(history.width > price.width && history.height > price.height);
    }
}
