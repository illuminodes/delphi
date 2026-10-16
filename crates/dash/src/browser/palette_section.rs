use yew::prelude::*;

use crate::{BrandCamp, BrandPalette, BrandSwatch};

#[function_component(PaletteSection)]
pub fn palette_section() -> Html {
    html! {
        <section>
            <h2>{"Palette"}</h2>
            <p class="lede">
                {"The two halves of the oracle. Apollo is the chain: the answer, given once and final. Pythia \
                  is the mempool: the priestess over the chasm, reading the vapours while the answer forms. \
                  The Ground colours sit underneath both."}
            </p>
            <div class="camps">
                {for BrandCamp::ALL.into_iter().map(CampCard::render)}
            </div>
        </section>
    }
}

struct CampCard;

impl CampCard {
    fn render(camp: BrandCamp) -> Html {
        html! {
            <article class={classes!("camp", camp.key())}>
                <h3>{camp.title()}</h3>
                <p>{camp.summary()}</p>
                <ul class="swatches">
                    {for BrandPalette::of(camp).map(Self::swatch)}
                </ul>
            </article>
        }
    }

    fn swatch(swatch: &BrandSwatch) -> Html {
        html! {
            <li class="swatch">
                <span class="chip" style={format!("background: var({})", swatch.css_var())}></span>
                <span>
                    <span class="name">{swatch.name}</span>
                    <span class="role">{swatch.role}</span>
                </span>
                <code>{swatch.hex}<span>{swatch.css_var()}</span></code>
            </li>
        }
    }
}
