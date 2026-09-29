use nostro2::NostrNote;

use crate::NoteSink;

/// Prints each note instead of sending it, for running without a relay.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdoutSink;

impl NoteSink for StdoutSink {
    fn deliver(&self, note: &NostrNote) -> Result<(), String> {
        let window = note.tags.find_tags_ref("d").first().copied().unwrap_or("?");
        println!("{window} {}", note.content);
        Ok(())
    }
}
