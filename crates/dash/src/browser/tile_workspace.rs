use web_sys::Element;
use yew::prelude::*;

use super::box_size_hook::use_box_size;
use super::pointer_reader::PointerReader;
use crate::{DividerSpot, PanelId, TileGesture, TileSize, WorkspaceAction, WorkspaceState};

#[derive(Properties, PartialEq)]
pub struct TileWorkspaceProps {
    pub render: Callback<PanelId, Html>,
}

/// Every card as a tile of the spiral: each card's inner edge drags to resize it, and a
/// tile's grip, a handle in its top-right corner, carries it onto another tile's edge (to move
/// it there) or centre (to swap). The cards name themselves, so the tile adds no title.
/// The canvas is the viewport, grown so no card falls below its minimum; the viewport scrolls.
#[function_component(TileWorkspace)]
pub fn tile_workspace(props: &TileWorkspaceProps) -> Html {
    let state = use_reducer(WorkspaceState::default);
    let viewport = use_node_ref();
    let surface = use_node_ref();
    let (width, height) = use_box_size(&viewport);

    {
        let state = state.clone();
        use_effect_with((width.round() as i64, height.round() as i64), move |_| {
            state.dispatch(WorkspaceAction::Viewport(TileSize::new(width, height)));
        });
    }

    let begin = {
        let state = state.clone();
        let surface = surface.clone();
        Callback::from(move |(event, gesture): (PointerEvent, TileGesture)| {
            if event.button() != 0 {
                return;
            }
            event.prevent_default();
            event.stop_propagation();
            if let Some(element) = surface.cast::<Element>() {
                let _ = element.set_pointer_capture(event.pointer_id());
            }
            state.dispatch(WorkspaceAction::Begin(gesture));
        })
    };

    let active = state.gesture().is_some();
    let on_move = {
        let state = state.clone();
        let surface = surface.clone();
        Callback::from(move |event: PointerEvent| {
            if !active {
                return;
            }
            let fraction = surface.cast::<Element>().and_then(|element| PointerReader::fraction_of(&event, &element));
            if let Some((x, y)) = fraction {
                state.dispatch(WorkspaceAction::Follow { x, y });
            }
        })
    };
    let on_end = {
        let state = state.clone();
        let surface = surface.clone();
        Callback::from(move |event: PointerEvent| {
            if !active {
                return;
            }
            if let Some(element) = surface.cast::<Element>() {
                let _ = element.release_pointer_capture(event.pointer_id());
            }
            state.dispatch(WorkspaceAction::Finish);
        })
    };

    let layout = state.layout();
    let carried = state.gesture().and_then(TileGesture::carried);
    let preview = state.gesture().and_then(|gesture| gesture.preview(layout));
    let gesture_class = state.gesture().map(TileGesture::css_class);
    let canvas = layout.canvas();
    let canvas_style = if canvas.width > 0.0 && canvas.height > 0.0 {
        format!("width:{:.0}px;height:{:.0}px", canvas.width, canvas.height)
    } else {
        "width:100%;height:100%".to_string()
    };

    html! {
        <div class="tiles-viewport" ref={viewport}>
        <div
            class={classes!("tiles", gesture_class)}
            style={canvas_style}
            ref={surface}
            onpointermove={on_move}
            onpointerup={on_end.clone()}
            onpointercancel={on_end.clone()}
            onlostpointercapture={on_end}
        >
            {for layout.tiles().into_iter().map(|(panel, rect)| {
                let grab = {
                    let begin = begin.clone();
                    Callback::from(move |event: PointerEvent| begin.emit((event, TileGesture::carrying(panel))))
                };
                let is_carried = carried == Some(&panel);
                html! {
                    <section key={panel.title()} class={classes!("tile", is_carried.then_some("carried"))} style={rect.style()}>
                        <div class="tile-frame">
                            <div
                                class="tile-grip"
                                onpointerdown={grab}
                                title={format!("Drag to move {}", panel.title())}
                                aria-label={format!("Move {}", panel.title())}
                            >
                                {"⠿"}
                            </div>
                            <div class="tile-body">{props.render.emit(panel)}</div>
                        </div>
                    </section>
                }
            })}
            {for layout.dividers().into_iter().map(|spot| TileWorkspaceDivider::render(spot, &begin))}
            {preview.map_or_else(Html::default, |rect| html! {
                <div class="tile-preview" style={rect.style()} />
            })}
        </div>
        </div>
    }
}

/// The draggable boundary of one split.
struct TileWorkspaceDivider;

impl TileWorkspaceDivider {
    fn render(spot: DividerSpot, begin: &Callback<(PointerEvent, TileGesture)>) -> Html {
        let class = classes!("tile-divider", spot.axis().css_class());
        let style = spot.style();
        let key = spot.key();
        let begin = begin.clone();
        let grab = Callback::from(move |event: PointerEvent| begin.emit((event, TileGesture::Resize(spot.clone()))));
        html! { <div key={key} class={class} style={style} role="separator" onpointerdown={grab} /> }
    }
}
