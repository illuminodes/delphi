use yew::prelude::*;

use super::chart_playground::ChartPlayground;
use super::component_gallery::ComponentGallery;
use super::delphi_mark::DelphiMark;
use super::meander_rule::MeanderRule;
use super::palette_section::PaletteSection;
use super::typeface_section::TypefaceSection;

#[function_component(BrandSheet)]
pub fn brand_sheet() -> Html {
    html! {
        <div class="delphi-sheet">
            <div class="page">
                <a class="back" href="#">{"← Dashboard"}</a>
                <header class="hero">
                    <DelphiMark class="mark" />
                    <h1 class="wordmark">{"DELPHI"}</h1>
                    <p class="tagline">{"The on-chain oracle"}</p>
                    <p class="inscription">{"ΓΝΩΘΙ ΣΕΑΥΤΟΝ"}</p>
                </header>
                <MeanderRule />
                <PaletteSection />
                <MeanderRule />
                <TypefaceSection />
                <MeanderRule />
                <ComponentGallery />
                <MeanderRule />
                <ChartPlayground />
                <footer>{"ΜΗΔΕΝ ΑΓΑΝ"}</footer>
            </div>
        </div>
    }
}
