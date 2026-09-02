#[derive(Debug, Clone)]
pub struct SmoothStencil {
    weights: Vec<f64>,
}

impl Default for SmoothStencil {
    fn default() -> Self {
        Self::new()
    }
}

impl SmoothStencil {
    pub const LEN: usize = 803;

    #[must_use]
    pub fn new() -> Self {
        let weights = (0_u32..803)
            .map(|x| {
                let distance = f64::from(x) - 411.0;
                0.001_50 * (-(distance * distance) / (2.0 * 201.0_f64.powi(2))).exp()
                    + 0.000_000_5 * f64::from(x)
            })
            .collect();
        Self { weights }
    }

    #[must_use]
    pub fn weights(&self) -> &[f64] {
        &self.weights
    }
}

#[derive(Debug, Clone)]
pub struct SpikeStencil {
    weights: Vec<f64>,
}

impl Default for SpikeStencil {
    fn default() -> Self {
        Self::new()
    }
}

impl SpikeStencil {
    pub const LEN: usize = 803;
    #[allow(clippy::unreadable_literal)]
    const SPIKES: [(usize, f64); 29] = [
        (40, 0.001300198324984352),
        (141, 0.001676746949820743),
        (201, 0.003468805546942046),
        (202, 0.001991977522512513),
        (236, 0.001905066647961839),
        (261, 0.003341772718156079),
        (262, 0.002588902624584287),
        (296, 0.002577893841190244),
        (297, 0.002733728814200412),
        (340, 0.003076117748975647),
        (341, 0.005613067550103145),
        (342, 0.003088253178535568),
        (400, 0.002918457489366139),
        (401, 0.006174500465286022),
        (402, 0.004417068070043504),
        (403, 0.002628663628020371),
        (436, 0.002858828161543839),
        (461, 0.004097463611984264),
        (462, 0.003345917406120509),
        (496, 0.002521467726855856),
        (497, 0.002784125730361008),
        (541, 0.003792850444811335),
        (601, 0.003688240815848247),
        (602, 0.002392400117402263),
        (636, 0.001280993059008106),
        (661, 0.001654665137536031),
        (662, 0.001395501347054946),
        (741, 0.001154279140906312),
        (801, 0.000832244504868709),
    ];

    #[must_use]
    pub fn new() -> Self {
        let mut weights = vec![0.0; Self::LEN];
        for (index, weight) in Self::SPIKES {
            weights[index] = weight;
        }
        Self { weights }
    }

    #[must_use]
    pub fn weights(&self) -> &[f64] {
        &self.weights
    }
}

#[derive(Debug, Clone, Default)]
pub struct PriceStencil {
    smooth: SmoothStencil,
    spike: SpikeStencil,
}

impl PriceStencil {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn score(&self, curve: &[f64], include_smooth: bool) -> f64 {
        let spike: f64 = curve
            .iter()
            .zip(self.spike.weights())
            .map(|(a, b)| a * b)
            .sum();
        if include_smooth {
            spike
                + 0.65
                    * curve
                        .iter()
                        .zip(self.smooth.weights())
                        .map(|(a, b)| a * b)
                        .sum::<f64>()
        } else {
            spike
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        SpikeStencil::LEN
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{PriceStencil, SmoothStencil, SpikeStencil};

    #[test]
    #[allow(clippy::unreadable_literal)]
    fn constructs_expected_stencils() {
        let smooth = SmoothStencil::new();
        let spike = SpikeStencil::new();
        assert_eq!(smooth.weights().len(), 803);
        assert_eq!(spike.weights().len(), 803);
        assert!((spike.weights()[401] - 0.006174500465286022).abs() < f64::EPSILON);
        assert!(spike.weights()[0].abs() < f64::EPSILON);
    }

    #[test]
    fn scores_aligned_spikes() {
        let stencil = PriceStencil::new();
        let mut curve = vec![0.0; stencil.len()];
        curve[401] = 1.0;
        assert!(stencil.score(&curve, false) > 0.006);
    }
}
