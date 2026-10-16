use yew::prelude::*;

use crate::BrandTypeface;

#[function_component(TypefaceSection)]
pub fn typeface_section() -> Html {
    html! {
        <section>
            <h2>{"Type"}</h2>
            <p class="lede">
                {"Flared small caps for display, a humanist sans for text, and a monospace for numbers."}
            </p>
            <div class="faces">
                {for BrandTypeface::ALL.iter().map(FaceCard::render)}
            </div>
            <ul class="scale">
                {for TypeScale::STEPS.iter().map(TypeScale::render)}
            </ul>
        </section>
    }
}

struct FaceCard;

impl FaceCard {
    fn render(face: &BrandTypeface) -> Html {
        let specimen_class = classes!("specimen", (face.role == "Display").then_some("display"));
        html! {
            <article class="face">
                <div class="face-meta">
                    <span class="role">{face.role}</span>
                    <span class="family">{face.family}</span>
                    <span class="note">{face.note}</span>
                    <code>{face.css_var}</code>
                </div>
                <p class={specimen_class} style={format!("font-family: {}", face.stack)}>{face.specimen}</p>
            </article>
        }
    }
}

struct TypeScale {
    class: &'static str,
    sample: &'static str,
}

impl TypeScale {
    const STEPS: [Self; 6] = [
        Self { class: "t-display", sample: "Delphi" },
        Self { class: "t-heading", sample: "Daily window" },
        Self { class: "t-figure", sample: "$00,000.00" },
        Self { class: "t-body", sample: "Median of the outputs that voted inside the band." },
        Self { class: "t-label", sample: "Blocks · Deviation · Hash" },
        Self { class: "t-mono", sample: "00000000000000000002a7c4…f19e" },
    ];

    fn render(&self) -> Html {
        html! {
            <li>
                <code>{format!(".{}", self.class)}</code>
                <span class={self.class}>{self.sample}</span>
            </li>
        }
    }
}
