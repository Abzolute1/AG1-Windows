//! Bounded preview-only port of partial-json 0.1.7's default parser.
//! See product/licenses/partial-json-port. Never use this to authorize a tool:
//! final arguments continue through the strict P06 reducer and P09 validation.
use ag1_core::{DomainError, resources::packages::validate_settings_document};
use serde_json::{Map, Value, json};

pub(crate) fn preview(input: &str) -> Result<Value, DomainError> {
    if input.len() > 512 * 1024 {
        return Err(DomainError::LimitExceeded("partial tool JSON bytes".into()));
    }
    let input = input.trim();
    if input.is_empty() {
        return Ok(json!({}));
    }
    let value = if let Ok(value) = serde_json::from_str::<Value>(input) {
        value
    } else {
        let mut parser = Parser {
            input,
            index: 0,
            nodes: 0,
        };
        parser
            .value(0)?
            .filter(|v| !v.is_null())
            .unwrap_or_else(|| json!({}))
    };
    validate_settings_document(&value)?;
    Ok(value)
}
struct Parser<'a> {
    input: &'a str,
    index: usize,
    nodes: usize,
}
impl Parser<'_> {
    fn byte(&self) -> Option<u8> {
        self.input.as_bytes().get(self.index).copied()
    }
    fn blanks(&mut self) {
        while self.byte().is_some_and(|c| b" \r\n\t".contains(&c)) {
            self.index += 1;
        }
    }
    fn value(&mut self, depth: usize) -> Result<Option<Value>, DomainError> {
        self.nodes += 1;
        if depth > 48 || self.nodes > 100000 {
            return Err(DomainError::LimitExceeded(
                "partial tool JSON complexity".into(),
            ));
        }
        self.blanks();
        let Some(byte) = self.byte() else {
            return Ok(None);
        };
        match byte {
            b'"' => Ok(self.string().map(Value::String)),
            b'{' => self.object(depth + 1).map(Some),
            b'[' => self.array(depth + 1).map(Some),
            _ => {
                let rest = &self.input[self.index..];
                for (literal, value) in [
                    ("null", Value::Null),
                    ("true", json!(true)),
                    ("false", json!(false)),
                ] {
                    if rest.starts_with(literal) || literal.starts_with(rest) {
                        self.index += literal.len();
                        return Ok(Some(value));
                    }
                }
                for literal in ["Infinity", "-Infinity", "NaN"] {
                    if rest.starts_with(literal) || (literal.starts_with(rest) && rest != "-") {
                        return Err(DomainError::InvalidInput(
                            "non-finite partial tool JSON".into(),
                        ));
                    }
                }
                Ok(self.number())
            }
        }
    }
    fn string(&mut self) -> Option<String> {
        let start = self.index;
        self.index += 1;
        let mut escape = false;
        while let Some(byte) = self.byte() {
            if byte == b'"' && !escape {
                self.index += 1;
                return serde_json::from_str(&self.input[start..self.index]).ok();
            }
            escape = byte == b'\\' && !escape;
            self.index += 1;
        }
        let end = self.index - usize::from(escape);
        let candidate = format!("{}\"", &self.input[start..end]);
        serde_json::from_str(&candidate).ok().or_else(|| {
            let slash = self.input[start..].rfind('\\')? + start;
            serde_json::from_str(&format!("{}\"", &self.input[start..slash])).ok()
        })
    }
    fn object(&mut self, depth: usize) -> Result<Value, DomainError> {
        self.index += 1;
        self.blanks();
        let mut object = Map::new();
        while self.byte() != Some(b'}') {
            self.blanks();
            if self.byte().is_none() {
                break;
            }
            let Some(key) = self.string() else {
                break;
            };
            self.blanks();
            // The pinned parser skips the separator without validating it. Keep
            // malformed-input compatibility while never splitting a UTF-8 scalar.
            self.index += self
                .input
                .get(self.index..)
                .and_then(|s| s.chars().next())
                .map(char::len_utf8)
                .unwrap_or(1);
            let Some(value) = self.value(depth)? else {
                break;
            };
            // SDK assigns into an ordinary object; __proto__ changes its prototype
            // instead of creating a serializable own property. No prototype exists here.
            if key != "__proto__" {
                object.insert(key, value);
            }
            self.blanks();
            if self.byte() == Some(b',') {
                self.index += 1;
            }
        }
        self.index += usize::from(self.byte() == Some(b'}'));
        Ok(Value::Object(object))
    }
    fn array(&mut self, depth: usize) -> Result<Value, DomainError> {
        self.index += 1;
        let mut values = vec![];
        while self.byte() != Some(b']') {
            let Some(value) = self.value(depth)? else {
                break;
            };
            values.push(value);
            self.blanks();
            if self.byte() == Some(b',') {
                self.index += 1;
            }
        }
        self.index += usize::from(self.byte() == Some(b']'));
        Ok(Value::Array(values))
    }
    fn number(&mut self) -> Option<Value> {
        let start = self.index;
        if start == 0 {
            self.index = self.input.len();
        } else {
            if self.byte() == Some(b'-') {
                self.index += 1;
            }
            while self.byte().is_some_and(|c| !b",]}".contains(&c)) {
                self.index += 1;
            }
        }
        let text = &self.input[start..self.index];
        serde_json::from_str(text).ok().or_else(|| {
            let exponent = self.input.rfind('e').filter(|&index| index >= start)?;
            serde_json::from_str(&self.input[start..exponent]).ok()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn js_numbers(value: &mut Value) {
        match value {
            Value::Number(number) => *value = json!(number.as_f64().unwrap()),
            Value::Array(values) => values.iter_mut().for_each(js_numbers),
            Value::Object(values) => values.values_mut().for_each(js_numbers),
            _ => {}
        }
    }
    #[test]
    fn all_pinned_partial_json_prefixes_match_the_sdk_preview() {
        let oracle: Value = serde_json::from_str(include_str!(
            "../../../../fixtures/tools/partial-json-oracle.json"
        ))
        .unwrap();
        assert_eq!(oracle["rows"].as_array().unwrap().len(), 269);
        for row in oracle["rows"].as_array().unwrap() {
            let input = row["input"].as_str().unwrap();
            if row["boundary"] == "nonfinite" {
                assert!(preview(input).is_err(), "{input:?}");
            } else {
                let mut actual = preview(input).unwrap();
                let mut expected = row["preview"].clone();
                // JS has one number type; JSON 400 and 400.0 carry the same value.
                js_numbers(&mut actual);
                js_numbers(&mut expected);
                assert_eq!(actual, expected, "{input:?}");
            }
        }
        for input in [
            "{\"a\"é}",
            "{\"a\"🦙1}",
            "{é\"value\"",
            "{\"x\":\"é\\",
            "{\"x\":\"🦙\\u12",
        ] {
            // Malformed separators and escape tails must not split UTF-8 or panic.
            let _ = preview(input);
        }
        assert!(preview(&"[".repeat(10000)).is_err());
        assert!(preview(&" ".repeat(512 * 1024 + 1)).is_err());
        assert!(preview(&format!("[{}", "0,".repeat(100001))).is_err());
    }
}
