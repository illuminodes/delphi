use yew::prelude::*;

#[function_component(ComponentGallery)]
pub fn component_gallery() -> Html {
    html! {
        <section>
            <h2>{"Components"}</h2>
            <p class="lede">
                {"Building blocks for the dashboard, shown empty. Corners are near-square, borders are thin \
                  and drawn in ash, and each card has a coloured top edge for its side."}
            </p>
            <div class="gallery">
                <div class="exhibit">
                    <h3>{"Buttons"}</h3>
                    <div class="row">
                        <button type="button" class="btn primary">{"Consult"}</button>
                        <button type="button" class="btn ghost">{"Settings"}</button>
                        <button type="button" class="btn danger">{"Forget relay"}</button>
                    </div>
                    <h3>{"Seals"}</h3>
                    <div class="row">
                        <span class="seal open">{"Connected"}</span>
                        <span class="seal connecting">{"Connecting"}</span>
                        <span class="seal closed">{"Disconnected"}</span>
                    </div>
                    <div class="row">
                        <span class="seal chain">{"Chain"}</span>
                        <span class="seal mempool">{"Mempool · live"}</span>
                    </div>
                    <h3>{"Change"}</h3>
                    <div class="row">
                        <span class="delta up">{"▲ +0.00%"}</span>
                        <span class="delta down">{"▼ −0.00%"}</span>
                    </div>
                </div>
                <div class="exhibit">
                    <h3>{"Tablets"}</h3>
                    <article class="tablet">
                        <header>
                            <h4>{"Daily"}</h4>
                            <span class="t-label">{"144 blocks"}</span>
                        </header>
                        <p class="figure">{"$––,–––"}</p>
                        <div class="rule"></div>
                    </article>
                    <article class="tablet mempool">
                        <header>
                            <h4>{"Mempool"}</h4>
                            <span class="seal mempool">{"Live"}</span>
                        </header>
                        <p class="figure">{"$––,–––"}</p>
                        <div class="rule"></div>
                    </article>
                </div>
                <div class="exhibit">
                    <h3>{"Fields"}</h3>
                    <label class="field">
                        <span class="t-label">{"Relay"}</span>
                        <input type="text" placeholder="wss://relay.example" />
                    </label>
                    <label class="field">
                        <span class="t-label">{"Publisher"}</span>
                        <input type="text" placeholder="npub1…" />
                    </label>
                </div>
            </div>
        </section>
    }
}
