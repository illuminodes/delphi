/// Anything the signer can turn into one addressable note.
pub trait NotePayload {
    /// The `d` tag, which names the window the note replaces.
    fn window(&self) -> &str;

    /// The `t` tag.
    fn label(&self) -> &str;

    /// The price the publish policy compares against the last publication.
    fn price(&self) -> f64;

    fn is_publishable(&self) -> bool;

    fn to_json(&self) -> String;
}
