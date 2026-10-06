use crate::NoteClass;

/// Anything the signer can turn into one note.
pub trait NotePayload {
    /// The window name, carried in the tag [`NoteClass::window_tag`] names.
    fn window(&self) -> &str;

    /// The `t` tag.
    fn label(&self) -> &str;

    /// The price the publish policy compares against the last publication.
    fn price(&self) -> f64;

    fn class(&self) -> NoteClass {
        NoteClass::Addressable
    }

    /// A fixed `created_at`, or `None` to stamp the note with the current time.
    fn created_at(&self) -> Option<i64> {
        None
    }

    fn extra_tags(&self) -> Vec<(&'static str, String)> {
        Vec::new()
    }

    fn is_publishable(&self) -> bool;

    fn to_json(&self) -> String;
}
