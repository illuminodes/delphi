#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewRoute {
    Dashboard,
    Brand,
    Etched,
    Carved,
    Fumes,
    Oracle,
}

impl ViewRoute {
    const BRAND_HASH: &'static str = "#delphi";
    const ETCHED_HASH: &'static str = "#etched";
    const CARVED_HASH: &'static str = "#carved";
    const FUMES_HASH: &'static str = "#fumes";
    const ORACLE_HASH: &'static str = "#oracle";

    #[must_use]
    pub fn from_hash(hash: &str) -> Self {
        if hash.eq_ignore_ascii_case(Self::BRAND_HASH) {
            Self::Brand
        } else if hash.eq_ignore_ascii_case(Self::ETCHED_HASH) {
            Self::Etched
        } else if hash.eq_ignore_ascii_case(Self::CARVED_HASH) {
            Self::Carved
        } else if hash.eq_ignore_ascii_case(Self::FUMES_HASH) {
            Self::Fumes
        } else if hash.eq_ignore_ascii_case(Self::ORACLE_HASH) {
            Self::Oracle
        } else {
            Self::Dashboard
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ViewRoute;

    #[test]
    fn the_delphi_hash_opens_the_brand_sheet() {
        assert_eq!(ViewRoute::from_hash("#delphi"), ViewRoute::Brand);
        assert_eq!(ViewRoute::from_hash("#Delphi"), ViewRoute::Brand);
    }

    #[test]
    fn the_etched_hash_opens_the_carved_wireframe() {
        assert_eq!(ViewRoute::from_hash("#etched"), ViewRoute::Etched);
        assert_eq!(ViewRoute::from_hash("#ETCHED"), ViewRoute::Etched);
    }

    #[test]
    fn the_carved_hash_opens_the_three_dimensional_slab() {
        assert_eq!(ViewRoute::from_hash("#carved"), ViewRoute::Carved);
        assert_eq!(ViewRoute::from_hash("#Carved"), ViewRoute::Carved);
    }

    #[test]
    fn the_fumes_hash_opens_the_mempool_fumes() {
        assert_eq!(ViewRoute::from_hash("#fumes"), ViewRoute::Fumes);
        assert_eq!(ViewRoute::from_hash("#FUMES"), ViewRoute::Fumes);
    }

    #[test]
    fn the_oracle_hash_opens_the_polygonal_wall() {
        assert_eq!(ViewRoute::from_hash("#oracle"), ViewRoute::Oracle);
        assert_eq!(ViewRoute::from_hash("#Oracle"), ViewRoute::Oracle);
    }

    #[test]
    fn anything_else_opens_the_dashboard() {
        assert_eq!(ViewRoute::from_hash(""), ViewRoute::Dashboard);
        assert_eq!(ViewRoute::from_hash("#"), ViewRoute::Dashboard);
        assert_eq!(ViewRoute::from_hash("#delphic"), ViewRoute::Dashboard);
    }
}
