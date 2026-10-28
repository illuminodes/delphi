use web_sys::Element;
use yew::prelude::*;

use crate::ChartLayout;

pub struct PointerReader;

impl PointerReader {
    /// The pointer's x in `layout` units across `plot`, the element the chart fills. Yew delegates
    /// events to its root, so the event's own current target is not the chart.
    pub fn chart_x(event: &MouseEvent, plot: &NodeRef, layout: &ChartLayout) -> Option<f64> {
        let rect = plot.cast::<Element>()?.get_bounding_client_rect();
        if rect.width() <= 0.0 {
            return None;
        }
        Some((f64::from(event.client_x()) - rect.left()) / rect.width() * layout.width)
    }

    /// The pointer as fractions of `element`'s width and height; outside the element is outside 0..1.
    pub fn fraction_of(event: &PointerEvent, element: &Element) -> Option<(f64, f64)> {
        let rect = element.get_bounding_client_rect();
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return None;
        }
        Some((
            (f64::from(event.client_x()) - rect.left()) / rect.width(),
            (f64::from(event.client_y()) - rect.top()) / rect.height(),
        ))
    }
}
