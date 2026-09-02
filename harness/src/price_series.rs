use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesError(String);

impl SeriesError {
    #[must_use]
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SeriesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SeriesError {}

pub struct PriceSeries<'a> {
    body: &'a str,
    cursor: usize,
}

impl<'a> PriceSeries<'a> {
    /// Parse the `prices` array of a `CoinGecko` market chart response into
    /// `(milliseconds, price)` pairs.
    ///
    /// # Errors
    /// Returns [`SeriesError`] if the body holds no `prices` array or a pair
    /// is malformed.
    pub fn parse(body: &'a str) -> Result<Vec<(u64, f64)>, SeriesError> {
        let mut parser = Self { body, cursor: 0 };
        parser.expect_literal("\"prices\"")?;
        parser.expect(b'[')?;
        let mut pairs = Vec::new();
        parser.skip_insignificant();
        if parser.peek() == Some(b']') {
            return Ok(pairs);
        }
        loop {
            pairs.push(parser.parse_pair()?);
            parser.skip_insignificant();
            match parser.peek() {
                Some(b',') => {
                    parser.cursor += 1;
                    parser.skip_insignificant();
                }
                Some(b']') => break,
                _ => {
                    return Err(SeriesError(
                        "expected ',' or ']' after a price pair".to_string(),
                    ))
                }
            }
        }
        Ok(pairs)
    }

    fn parse_pair(&mut self) -> Result<(u64, f64), SeriesError> {
        self.expect(b'[')?;
        let milliseconds = self.parse_timestamp()?;
        self.expect(b',')?;
        let price = self.parse_number("price")?;
        self.expect(b']')?;
        Ok((milliseconds, price))
    }

    fn parse_timestamp(&mut self) -> Result<u64, SeriesError> {
        let token = self.parse_token("timestamp")?;
        token
            .parse::<u64>()
            .map_err(|_| SeriesError(format!("invalid timestamp literal: {token}")))
    }

    fn parse_number(&mut self, field: &str) -> Result<f64, SeriesError> {
        let token = self.parse_token(field)?;
        token
            .parse::<f64>()
            .map_err(|_| SeriesError(format!("invalid {field} literal: {token}")))
    }

    fn parse_token(&mut self, field: &str) -> Result<&'a str, SeriesError> {
        self.skip_insignificant();
        let bytes = self.body.as_bytes();
        let start = self.cursor;
        while let Some(&byte) = bytes.get(self.cursor) {
            if byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'+' | b'e' | b'E') {
                self.cursor += 1;
            } else {
                break;
            }
        }
        if start == self.cursor {
            return Err(SeriesError(format!("missing {field} literal")));
        }
        Ok(&self.body[start..self.cursor])
    }

    fn expect(&mut self, delimiter: u8) -> Result<(), SeriesError> {
        self.skip_insignificant();
        if self.peek() == Some(delimiter) {
            self.cursor += 1;
            Ok(())
        } else {
            Err(SeriesError(format!(
                "expected '{}' at byte {}",
                delimiter as char, self.cursor
            )))
        }
    }

    fn expect_literal(&mut self, literal: &str) -> Result<(), SeriesError> {
        let found = self.body[self.cursor..]
            .find(literal)
            .ok_or_else(|| SeriesError(format!("no {literal} array found")))?;
        self.cursor += found + literal.len();
        self.skip_insignificant();
        while self.peek().is_some_and(|byte| byte != b'[') {
            self.cursor += 1;
        }
        if self.peek() == Some(b'[') {
            Ok(())
        } else {
            Err(SeriesError(format!("no array follows {literal}")))
        }
    }

    fn peek(&self) -> Option<u8> {
        self.body.as_bytes().get(self.cursor).copied()
    }

    fn skip_insignificant(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.cursor += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PriceSeries;

    #[test]
    fn parses_price_pairs_from_a_range_response() {
        let body = concat!(
            "{\"prices\":[[1788249600000,77000.5],",
            "[1788336000000,77370.25],[1788422400000,78001]]}"
        );
        let pairs = PriceSeries::parse(body).expect("well-formed body");
        assert_eq!(
            pairs,
            [
                (1_788_249_600_000, 77_000.5),
                (1_788_336_000_000, 77_370.25),
                (1_788_422_400_000, 78_001.0)
            ]
        );
    }

    #[test]
    fn parses_an_empty_prices_array() {
        let pairs = PriceSeries::parse("{\"prices\":[]}").expect("empty array is well-formed");
        assert!(pairs.is_empty());
    }

    #[test]
    fn rejects_a_body_without_prices() {
        assert!(PriceSeries::parse("{\"market_caps\":[[1,2]]}").is_err());
        assert!(PriceSeries::parse("{\"prices\":[[1788249600000]]}").is_err());
    }

    #[test]
    fn parses_pairs_with_whitespace_and_exponents() {
        let body = "{ \"prices\" : [ [ 1788249600000 , 7.7e4 ] ] }";
        let pairs = PriceSeries::parse(body).expect("whitespace is insignificant");
        assert_eq!(pairs, [(1_788_249_600_000, 77_000.0)]);
    }
}
