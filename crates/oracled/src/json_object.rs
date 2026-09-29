use std::fmt::Write;

/// Writes a flat JSON object with string, integer, float and pre-encoded members.
pub struct JsonObject {
    text: String,
    members: usize,
}

impl JsonObject {
    #[must_use]
    pub fn new() -> Self {
        Self {
            text: String::from("{"),
            members: 0,
        }
    }

    fn key(&mut self, key: &str) {
        if self.members > 0 {
            self.text.push(',');
        }
        self.members += 1;
        self.quoted(key);
        self.text.push(':');
    }

    fn quoted(&mut self, value: &str) {
        self.text.push('"');
        for character in value.chars() {
            match character {
                '"' => self.text.push_str("\\\""),
                '\\' => self.text.push_str("\\\\"),
                control if control.is_control() => {
                    let _ = write!(self.text, "\\u{:04x}", u32::from(control));
                }
                other => self.text.push(other),
            }
        }
        self.text.push('"');
    }

    pub fn text(&mut self, key: &str, value: &str) {
        self.key(key);
        self.quoted(value);
    }

    pub fn count(&mut self, key: &str, value: usize) {
        self.key(key);
        self.text.push_str(&value.to_string());
    }

    pub fn float(&mut self, key: &str, value: f64) {
        self.key(key);
        if value.is_finite() {
            let _ = write!(self.text, "{value}");
        } else {
            self.text.push_str("null");
        }
    }

    /// Adds a member whose value is already valid JSON.
    pub fn raw(&mut self, key: &str, json: &str) {
        self.key(key);
        self.text.push_str(json);
    }

    #[must_use]
    pub fn finish(mut self) -> String {
        self.text.push('}');
        self.text
    }
}

impl Default for JsonObject {
    fn default() -> Self {
        Self::new()
    }
}
