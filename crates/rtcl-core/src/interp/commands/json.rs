//! JSON extension — jimtcl-compatible `json::decode` and `json::encode`.
//!
//! ## Commands
//!
//! - `json::decode ?-index? ?-null string? ?-schema? json-string`
//! - `json::encode value ?schema?`
//!
//! Also available as ensemble: `json decode ...`, `json encode ...`.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::{DictMap, Value};

// ── Public entry points ────────────────────────────────────────

/// `json` ensemble — dispatches to `decode` / `encode`.
pub fn cmd_json(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("json", 2, args.len()));
    }
    match args[1].as_str() {
        "decode" => cmd_json_decode(interp, args),
        "encode" => cmd_json_encode(interp, args),
        other => Err(Error::runtime(
            format!("unknown json subcommand \"{}\": must be decode or encode", other),
            ErrorCode::InvalidOp,
        )),
    }
}

/// `json::decode ?-index? ?-null string? ?-schema? json-string`
pub fn cmd_json_decode(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Parse options — skip args[0] ("json") and args[1] ("decode")
    let start = if args.len() > 1 && args[1].as_str() == "decode" { 2 } else { 1 };
    let mut index_mode = false;
    let mut null_value = "null".to_string();
    let mut schema_mode = false;
    let mut i = start;

    while i < args.len() {
        match args[i].as_str() {
            "-index" => { index_mode = true; i += 1; }
            "-null" => {
                i += 1;
                if i >= args.len() {
                    return Err(Error::runtime(
                        "-null requires a value", ErrorCode::InvalidOp));
                }
                null_value = args[i].as_str().to_string();
                i += 1;
            }
            "-schema" => { schema_mode = true; i += 1; }
            _ => break,
        }
    }
    if i >= args.len() {
        return Err(Error::wrong_args_with_usage(
            "json::decode", 1, 0, "?-index? ?-null string? ?-schema? json-string"));
    }
    let json_str = args[i].as_str();

    if json_str.is_empty() {
        return Err(Error::runtime("empty JSON string", ErrorCode::InvalidOp));
    }

    let mut parser = JsonParser::new(json_str);
    let (value, schema) = parser.parse_root(&null_value, index_mode)?;

    if schema_mode {
        Ok(Value::from_list_cached(vec![value, Value::from_str(&schema)]))
    } else {
        Ok(value)
    }
}

/// `json::encode value ?schema?`
pub fn cmd_json_encode(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let start = if args.len() > 1 && args[1].as_str() == "encode" { 2 } else { 1 };
    if args.len() <= start {
        return Err(Error::wrong_args_with_usage(
            "json::encode", 1, 0, "value ?schema?"));
    }
    let value = &args[start];
    let schema_str = if args.len() > start + 1 {
        args[start + 1].as_str().to_string()
    } else {
        "str".to_string()
    };

    let schema = parse_schema(&schema_str);
    let mut buf = String::new();
    encode_value(value, &schema, &mut buf)?;
    Ok(Value::from_str(&buf))
}

// ═══════════════════════════════════════════════════════════════
// JSON DECODER
// ═══════════════════════════════════════════════════════════════

struct JsonParser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(s: &'a str) -> Self {
        JsonParser { input: s.as_bytes(), pos: 0 }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() {
            match self.input[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.input.get(self.pos).copied()
    }

    fn parse_root(&mut self, null_val: &str, index_mode: bool) -> Result<(Value, String)> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(null_val, index_mode),
            Some(b'[') => self.parse_array(null_val, index_mode),
            Some(_) => Err(Error::runtime(
                "root element must be an object or an array", ErrorCode::InvalidOp)),
            None => Err(Error::runtime("empty JSON string", ErrorCode::InvalidOp)),
        }
    }

    fn parse_value(&mut self, null_val: &str, index_mode: bool) -> Result<(Value, String)> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(null_val, index_mode),
            Some(b'[') => self.parse_array(null_val, index_mode),
            Some(b'"') => {
                let s = self.parse_string()?;
                Ok((Value::from_str(&s), "str".to_string()))
            }
            Some(b't') => self.parse_literal("true", "true", "bool"),
            Some(b'f') => self.parse_literal("false", "false", "bool"),
            Some(b'n') => self.parse_literal("null", null_val, "num"),
            Some(b'I') => self.parse_literal("Infinity", "Inf", "num"),
            Some(b'N') => self.parse_literal("NaN", "NaN", "num"),
            Some(b'-') => {
                // Could be negative number or -Infinity
                if self.pos + 1 < self.input.len() && self.input[self.pos + 1] == b'I' {
                    self.parse_literal("-Infinity", "-Inf", "num")
                } else {
                    self.parse_number()
                }
            }
            Some(c) if c == b'+' || c.is_ascii_digit() => self.parse_number(),
            Some(c) => Err(Error::runtime(
                format!("invalid JSON: unexpected character '{}'", c as char),
                ErrorCode::InvalidOp)),
            None => Err(Error::runtime("truncated JSON string", ErrorCode::InvalidOp)),
        }
    }

    fn parse_object(&mut self, null_val: &str, index_mode: bool) -> Result<(Value, String)> {
        self.expect(b'{')?;
        let mut entries = DictMap::ordered();
        let mut schema_parts: Vec<String> = Vec::new();

        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok((Value::from_dict_cached(entries), "obj".to_string()));
        }

        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(b':')?;
            let (val, val_schema) = self.parse_value(null_val, index_mode)?;
            entries.insert(key.clone(), val);
            schema_parts.push(key);
            schema_parts.push(val_schema);

            self.skip_ws();
            match self.peek() {
                Some(b',') => { self.pos += 1; }
                Some(b'}') => { self.pos += 1; break; }
                _ => return Err(Error::runtime(
                    "invalid JSON string", ErrorCode::InvalidOp)),
            }
        }

        let schema = if schema_parts.is_empty() {
            "obj".to_string()
        } else {
            let mut s = String::from("obj");
            for part in &schema_parts {
                s.push(' ');
                if part.contains(' ') || part.contains('{') || part.contains('}') {
                    s.push('{');
                    s.push_str(part);
                    s.push('}');
                } else {
                    s.push_str(part);
                }
            }
            s
        };

        Ok((Value::from_dict_cached(entries), schema))
    }

    fn parse_array(&mut self, null_val: &str, index_mode: bool) -> Result<(Value, String)> {
        self.expect(b'[')?;
        let mut items: Vec<Value> = Vec::new();
        let mut schemas: Vec<String> = Vec::new();

        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return if index_mode {
                Ok((Value::from_dict_cached(DictMap::ordered()), "list".to_string()))
            } else {
                Ok((Value::from_list_cached(vec![]), "list".to_string()))
            };
        }

        loop {
            let (val, val_schema) = self.parse_value(null_val, index_mode)?;
            items.push(val);
            schemas.push(val_schema);

            self.skip_ws();
            match self.peek() {
                Some(b',') => { self.pos += 1; }
                Some(b']') => { self.pos += 1; break; }
                _ => return Err(Error::runtime(
                    "invalid JSON string", ErrorCode::InvalidOp)),
            }
        }

        // Determine schema
        let schema = if schemas.is_empty() {
            "list".to_string()
        } else {
            let first = &schemas[0];
            let all_same = schemas.iter().all(|s| s == first);
            if all_same && !first.starts_with("obj") && !first.starts_with("list")
                && !first.starts_with("mixed")
            {
                format!("list {}", first)
            } else {
                // Mixed
                let mut s = String::from("mixed");
                for sch in &schemas {
                    s.push(' ');
                    if sch.contains(' ') {
                        s.push('{');
                        s.push_str(sch);
                        s.push('}');
                    } else {
                        s.push_str(sch);
                    }
                }
                s
            }
        };

        if index_mode {
            // Convert array to dict with integer keys
            let mut dict = DictMap::ordered_with_capacity(items.len());
            for (idx, val) in items.into_iter().enumerate() {
                dict.insert(idx.to_string(), val);
            }
            Ok((Value::from_dict_cached(dict), schema))
        } else {
            Ok((Value::from_list_cached(items), schema))
        }
    }

    fn parse_string(&mut self) -> Result<String> {
        self.skip_ws();
        self.expect(b'"')?;
        let mut s = String::new();

        loop {
            if self.pos >= self.input.len() {
                return Err(Error::runtime("truncated JSON string", ErrorCode::InvalidOp));
            }
            let b = self.input[self.pos];
            match b {
                b'"' => { self.pos += 1; return Ok(s); }
                b'\\' => {
                    self.pos += 1;
                    if self.pos >= self.input.len() {
                        return Err(Error::runtime(
                            "truncated JSON string", ErrorCode::InvalidOp));
                    }
                    match self.input[self.pos] {
                        b'"' => s.push('"'),
                        b'\\' => s.push('\\'),
                        b'/' => s.push('/'),
                        b'b' => s.push('\u{08}'),
                        b'f' => s.push('\u{0C}'),
                        b'n' => s.push('\n'),
                        b'r' => s.push('\r'),
                        b't' => s.push('\t'),
                        b'u' => {
                            self.pos += 1;
                            let cp = self.parse_hex4()?;
                            // Handle UTF-16 surrogate pairs
                            if (0xD800..=0xDBFF).contains(&cp) {
                                // High surrogate — expect \uXXXX low surrogate
                                if self.pos + 1 < self.input.len()
                                    && self.input[self.pos] == b'\\'
                                    && self.input[self.pos + 1] == b'u'
                                {
                                    self.pos += 2;
                                    let low = self.parse_hex4()?;
                                    if (0xDC00..=0xDFFF).contains(&low) {
                                        let combined = 0x10000
                                            + ((cp as u32 - 0xD800) << 10)
                                            + (low as u32 - 0xDC00);
                                        if let Some(c) = char::from_u32(combined) {
                                            s.push(c);
                                        }
                                    }
                                }
                            } else if let Some(c) = char::from_u32(cp as u32) {
                                s.push(c);
                            }
                            continue; // Don't advance pos again
                        }
                        c => { s.push('\\'); s.push(c as char); }
                    }
                    self.pos += 1;
                }
                _ => {
                    // UTF-8 passthrough
                    s.push(b as char);
                    self.pos += 1;
                }
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u16> {
        if self.pos + 4 > self.input.len() {
            return Err(Error::runtime(
                "truncated unicode escape", ErrorCode::InvalidOp));
        }
        let hex_str = std::str::from_utf8(&self.input[self.pos..self.pos + 4])
            .map_err(|_| Error::runtime("invalid unicode escape", ErrorCode::InvalidOp))?;
        let val = u16::from_str_radix(hex_str, 16)
            .map_err(|_| Error::runtime(
                format!("invalid unicode escape: \\u{}", hex_str), ErrorCode::InvalidOp))?;
        self.pos += 4;
        Ok(val)
    }

    fn parse_number(&mut self) -> Result<(Value, String)> {
        let start = self.pos;
        // Consume sign
        if self.pos < self.input.len()
            && (self.input[self.pos] == b'-' || self.input[self.pos] == b'+')
        {
            self.pos += 1;
        }
        // Consume digits
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        // Fraction
        if self.pos < self.input.len() && self.input[self.pos] == b'.' {
            self.pos += 1;
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        // Exponent
        if self.pos < self.input.len()
            && (self.input[self.pos] == b'e' || self.input[self.pos] == b'E')
        {
            self.pos += 1;
            if self.pos < self.input.len()
                && (self.input[self.pos] == b'+' || self.input[self.pos] == b'-')
            {
                self.pos += 1;
            }
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }

        if self.pos == start {
            return Err(Error::runtime("invalid JSON number", ErrorCode::InvalidOp));
        }

        let num_str = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| Error::runtime("invalid JSON number", ErrorCode::InvalidOp))?;

        // Preserve original representation (jimtcl compat)
        Ok((Value::from_str(num_str), "num".to_string()))
    }

    fn parse_literal(&mut self, expected: &str, tcl_value: &str, schema: &str) -> Result<(Value, String)> {
        let eb = expected.as_bytes();
        if self.pos + eb.len() > self.input.len()
            || &self.input[self.pos..self.pos + eb.len()] != eb
        {
            return Err(Error::runtime("invalid JSON string", ErrorCode::InvalidOp));
        }
        self.pos += eb.len();
        Ok((Value::from_str(tcl_value), schema.to_string()))
    }

    fn expect(&mut self, byte: u8) -> Result<()> {
        self.skip_ws();
        if self.pos < self.input.len() && self.input[self.pos] == byte {
            self.pos += 1;
            Ok(())
        } else {
            Err(Error::runtime("invalid JSON string", ErrorCode::InvalidOp))
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// JSON ENCODER
// ═══════════════════════════════════════════════════════════════

/// Schema types for encoding.
#[derive(Debug, Clone)]
enum Schema {
    Str,
    Num,
    Bool,
    Obj(Vec<(String, Schema)>),  // named fields + optional wildcard (key="*")
    List(Box<Schema>),
    Mixed(Vec<Schema>),
}

/// Parse a schema string into a Schema tree.
fn parse_schema(s: &str) -> Schema {
    let s = s.trim();
    if s.is_empty() || s == "str" { return Schema::Str; }
    if s == "num" { return Schema::Num; }
    if s == "bool" { return Schema::Bool; }

    // Peel off leading keyword
    let (keyword, rest) = split_first_word(s);
    match keyword {
        "obj" => {
            let fields = parse_schema_fields(rest);
            Schema::Obj(fields)
        }
        "list" => {
            if rest.is_empty() {
                Schema::List(Box::new(Schema::Str))
            } else {
                Schema::List(Box::new(parse_schema(rest)))
            }
        }
        "mixed" => {
            let parts = split_tcl_words(rest);
            Schema::Mixed(parts.into_iter().map(|p| parse_schema(&p)).collect())
        }
        "str" => Schema::Str,
        "num" => Schema::Num,
        "bool" => Schema::Bool,
        _ => Schema::Str,
    }
}

fn parse_schema_fields(s: &str) -> Vec<(String, Schema)> {
    let words = split_tcl_words(s);
    let mut fields = Vec::new();
    let mut i = 0;
    while i + 1 < words.len() {
        let name = words[i].clone();
        let schema = parse_schema(&words[i + 1]);
        fields.push((name, schema));
        i += 2;
    }
    fields
}

/// Split a string into first word and remainder.
fn split_first_word(s: &str) -> (&str, &str) {
    let s = s.trim();
    if let Some(pos) = s.find(|c: char| c.is_whitespace()) {
        (&s[..pos], s[pos..].trim_start())
    } else {
        (s, "")
    }
}

/// Split a string into Tcl-like words (respecting braces).
fn split_tcl_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() { i += 1; }
        if i >= chars.len() { break; }

        if chars[i] == '{' {
            // Braced word
            i += 1;
            let mut depth = 1;
            let mut word = String::new();
            while i < chars.len() && depth > 0 {
                match chars[i] {
                    '{' => { depth += 1; word.push('{'); }
                    '}' => { depth -= 1; if depth > 0 { word.push('}'); } }
                    c => word.push(c),
                }
                i += 1;
            }
            words.push(word);
        } else {
            // Unbraced word
            let mut word = String::new();
            while i < chars.len() && !chars[i].is_whitespace() {
                word.push(chars[i]);
                i += 1;
            }
            words.push(word);
        }
    }

    words
}

fn encode_value(value: &Value, schema: &Schema, buf: &mut String) -> Result<()> {
    match schema {
        Schema::Str => encode_string(value.as_str(), buf),
        Schema::Num => encode_num(value.as_str(), buf),
        Schema::Bool => encode_bool(value, buf),
        Schema::Obj(fields) => encode_object(value, fields, buf),
        Schema::List(elem_schema) => encode_list(value, elem_schema, buf),
        Schema::Mixed(schemas) => encode_mixed(value, schemas, buf),
    }
    Ok(())
}

fn encode_string(s: &str, buf: &mut String) {
    buf.push('"');
    for c in s.chars() {
        match c {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '/' => buf.push_str("\\/"),
            '\u{08}' => buf.push_str("\\b"),
            '\u{0C}' => buf.push_str("\\f"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            c if c < '\u{20}' => {
                buf.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => buf.push(c),
        }
    }
    buf.push('"');
}

fn encode_num(s: &str, buf: &mut String) {
    let trimmed = s.trim();
    match trimmed {
        "Inf" | "inf" => buf.push_str("Infinity"),
        "-Inf" | "-inf" => buf.push_str("-Infinity"),
        _ => buf.push_str(trimmed),
    }
}

fn encode_bool(value: &Value, buf: &mut String) {
    if let Some(b) = value.as_bool() {
        buf.push_str(if b { "true" } else { "false" });
    } else {
        // Truthy fallback: non-empty and not "0"/"false"/"no"/"off" → true
        buf.push_str(if value.is_true() { "true" } else { "false" });
    }
}

fn encode_object(value: &Value, fields: &[(String, Schema)], buf: &mut String) {
    let items = value.as_list().unwrap_or_default();
    // Parse as dict pairs
    let mut dict = DictMap::ordered();
    for chunk in items.chunks(2) {
        if chunk.len() == 2 {
            dict.insert(chunk[0].as_str().to_string(), chunk[1].clone());
        }
    }

    // Also try as_dict directly
    if dict.is_empty() {
        if let Some(d) = value.as_dict() {
            dict = d;
        }
    }

    // Sort keys alphabetically (jimtcl compat)
    let mut sorted_keys: Vec<String> = dict.keys().map(|k| k.clone()).collect();
    sorted_keys.sort();

    // Build field lookup for named schemas
    let mut field_map = std::collections::HashMap::new();
    let mut wildcard_schema: Option<&Schema> = None;
    for (name, schema) in fields {
        if name == "*" {
            wildcard_schema = Some(schema);
        } else {
            field_map.insert(name.as_str(), schema);
        }
    }

    buf.push('{');
    let mut first = true;
    for key in &sorted_keys {
        if !first { buf.push_str(", "); }
        first = false;
        encode_string(key, buf);
        buf.push_str(": ");
        if let Some(val) = dict.get(key) {
            let schema = field_map.get(key.as_str())
                .copied()
                .or(wildcard_schema)
                .unwrap_or(&Schema::Str);
            let _ = encode_value(val, schema, buf);
        }
    }
    buf.push('}');
}

fn encode_list(value: &Value, elem_schema: &Schema, buf: &mut String) {
    let items = value.as_list().unwrap_or_default();
    buf.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 { buf.push_str(", "); }
        let _ = encode_value(item, elem_schema, buf);
    }
    buf.push(']');
}

fn encode_mixed(value: &Value, schemas: &[Schema], buf: &mut String) {
    let items = value.as_list().unwrap_or_default();
    buf.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 { buf.push_str(", "); }
        let schema = schemas.get(i).unwrap_or(&Schema::Str);
        let _ = encode_value(item, schema, buf);
    }
    buf.push(']');
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    /// Helper: set a variable to a JSON string, then decode it.
    /// This avoids Tcl quoting issues with braces/brackets in JSON.
    fn decode(interp: &mut Interp, json: &str) -> crate::error::Result<crate::value::Value> {
        interp.set_var("_json_", crate::value::Value::from_str(json))?;
        interp.eval("json::decode $_json_")
    }

    fn decode_opts(interp: &mut Interp, opts: &str, json: &str) -> crate::error::Result<crate::value::Value> {
        interp.set_var("_json_", crate::value::Value::from_str(json))?;
        interp.eval(&format!("json::decode {} $_json_", opts))
    }

    // ── Decode tests ───────────────────────────────────────────

    #[test]
    fn test_decode_empty_object() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, "{}").unwrap();
        assert_eq!(r.as_str(), "");
    }

    #[test]
    fn test_decode_empty_array() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, "[]").unwrap();
        assert_eq!(r.as_str(), "");
    }

    #[test]
    fn test_decode_simple_object() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, r#"{"name":"Jim","age":30}"#).unwrap();
        interp.set_var("d", r).unwrap();
        assert_eq!(interp.eval("dict get $d name").unwrap().as_str(), "Jim");
        assert_eq!(interp.eval("dict get $d age").unwrap().as_str(), "30");
    }

    #[test]
    fn test_decode_simple_array() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, "[1, 2, 3]").unwrap();
        interp.set_var("r", r).unwrap();
        assert_eq!(interp.eval("llength $r").unwrap().as_str(), "3");
        assert_eq!(interp.eval("lindex $r 0").unwrap().as_str(), "1");
        assert_eq!(interp.eval("lindex $r 2").unwrap().as_str(), "3");
    }

    #[test]
    fn test_decode_string_values() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, r#"{"greeting":"hello world"}"#).unwrap();
        interp.set_var("d", r).unwrap();
        assert_eq!(interp.eval("dict get $d greeting").unwrap().as_str(), "hello world");
    }

    #[test]
    fn test_decode_nested_object() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, r#"{"a":{"x":10,"y":20}}"#).unwrap();
        interp.set_var("d", r).unwrap();
        assert_eq!(interp.eval("dict get $d a x").unwrap().as_str(), "10");
        assert_eq!(interp.eval("dict get $d a y").unwrap().as_str(), "20");
    }

    #[test]
    fn test_decode_bool_values() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, r#"{"on":true,"off":false}"#).unwrap();
        interp.set_var("d", r).unwrap();
        assert_eq!(interp.eval("dict get $d on").unwrap().as_str(), "true");
        assert_eq!(interp.eval("dict get $d off").unwrap().as_str(), "false");
    }

    #[test]
    fn test_decode_null_default() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, r#"{"val":null}"#).unwrap();
        interp.set_var("d", r).unwrap();
        assert_eq!(interp.eval("dict get $d val").unwrap().as_str(), "null");
    }

    #[test]
    fn test_decode_null_custom() {
        let mut interp = Interp::new();
        let r = decode_opts(&mut interp, "-null NULL", r#"{"val":null}"#).unwrap();
        interp.set_var("d", r).unwrap();
        assert_eq!(interp.eval("dict get $d val").unwrap().as_str(), "NULL");
    }

    #[test]
    fn test_decode_unicode_escape() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, r#"{"key":"\u2022"}"#).unwrap();
        interp.set_var("d", r).unwrap();
        assert_eq!(interp.eval("dict get $d key").unwrap().as_str(), "\u{2022}");
    }

    #[test]
    fn test_decode_backslash_escapes() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, r#"{"a":"line1\nline2","b":"tab\there"}"#).unwrap();
        interp.set_var("d", r).unwrap();
        assert_eq!(interp.eval("dict get $d a").unwrap().as_str(), "line1\nline2");
        assert_eq!(interp.eval("dict get $d b").unwrap().as_str(), "tab\there");
    }

    #[test]
    fn test_decode_number_forms() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, "[1, 2, 3.0, 1e5, -1e-5]").unwrap();
        interp.set_var("r", r).unwrap();
        assert_eq!(interp.eval("lindex $r 0").unwrap().as_str(), "1");
        assert_eq!(interp.eval("lindex $r 2").unwrap().as_str(), "3.0");
    }

    #[test]
    fn test_decode_infinity_nan() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, "[Infinity, -Infinity, NaN]").unwrap();
        interp.set_var("r", r).unwrap();
        assert_eq!(interp.eval("lindex $r 0").unwrap().as_str(), "Inf");
        assert_eq!(interp.eval("lindex $r 1").unwrap().as_str(), "-Inf");
        assert_eq!(interp.eval("lindex $r 2").unwrap().as_str(), "NaN");
    }

    #[test]
    fn test_decode_schema_mode() {
        let mut interp = Interp::new();
        let r = decode_opts(&mut interp, "-schema", r#"{"a":1,"b":"hello"}"#).unwrap();
        interp.set_var("result", r).unwrap();
        let schema = interp.eval("lindex $result 1").unwrap();
        assert!(schema.as_str().contains("obj"), "schema should contain 'obj': {}", schema.as_str());
    }

    #[test]
    fn test_decode_schema_array_homogeneous() {
        let mut interp = Interp::new();
        let r = decode_opts(&mut interp, "-schema", "[1, 2, 3]").unwrap();
        interp.set_var("result", r).unwrap();
        let schema = interp.eval("lindex $result 1").unwrap();
        assert_eq!(schema.as_str(), "list num");
    }

    #[test]
    fn test_decode_schema_array_mixed() {
        let mut interp = Interp::new();
        let r = decode_opts(&mut interp, "-schema", r#"[1, "hello"]"#).unwrap();
        interp.set_var("result", r).unwrap();
        let schema = interp.eval("lindex $result 1").unwrap();
        assert_eq!(schema.as_str(), "mixed num str");
    }

    #[test]
    fn test_decode_index_mode() {
        let mut interp = Interp::new();
        let r = decode_opts(&mut interp, "-index", "[10, 20, 30]").unwrap();
        interp.set_var("d", r).unwrap();
        assert_eq!(interp.eval("dict get $d 0").unwrap().as_str(), "10");
        assert_eq!(interp.eval("dict get $d 1").unwrap().as_str(), "20");
        assert_eq!(interp.eval("dict get $d 2").unwrap().as_str(), "30");
    }

    #[test]
    fn test_decode_error_empty() {
        let mut interp = Interp::new();
        assert!(decode(&mut interp, "").is_err());
    }

    #[test]
    fn test_decode_error_bare_value() {
        let mut interp = Interp::new();
        assert!(decode(&mut interp, "42").is_err());
    }

    #[test]
    fn test_decode_error_invalid() {
        let mut interp = Interp::new();
        assert!(decode(&mut interp, r#"{"key"}"#).is_err());
    }

    #[test]
    fn test_decode_array_of_objects() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, r#"[{"a":1},{"b":2}]"#).unwrap();
        interp.set_var("r", r).unwrap();
        assert_eq!(interp.eval("dict get [lindex $r 0] a").unwrap().as_str(), "1");
        assert_eq!(interp.eval("dict get [lindex $r 1] b").unwrap().as_str(), "2");
    }

    #[test]
    fn test_decode_deeply_nested() {
        let mut interp = Interp::new();
        let r = decode(&mut interp, r#"{"a":{"b":{"c":42}}}"#).unwrap();
        interp.set_var("d", r).unwrap();
        assert_eq!(interp.eval("dict get $d a b c").unwrap().as_str(), "42");
    }

    // ── Encode tests ───────────────────────────────────────────

    #[test]
    fn test_encode_string_default() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"json::encode "hello world""#).unwrap();
        assert_eq!(r.as_str(), r#""hello world""#);
    }

    #[test]
    fn test_encode_num() {
        let mut interp = Interp::new();
        let r = interp.eval("json::encode 42 num").unwrap();
        assert_eq!(r.as_str(), "42");
    }

    #[test]
    fn test_encode_bool_true() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("json::encode 1 bool").unwrap().as_str(), "true");
        assert_eq!(interp.eval("json::encode yes bool").unwrap().as_str(), "true");
    }

    #[test]
    fn test_encode_bool_false() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("json::encode 0 bool").unwrap().as_str(), "false");
        assert_eq!(interp.eval("json::encode no bool").unwrap().as_str(), "false");
    }

    #[test]
    fn test_encode_list_of_strings() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"json::encode {a b c} list"#).unwrap();
        assert_eq!(r.as_str(), r#"["a", "b", "c"]"#);
    }

    #[test]
    fn test_encode_list_of_nums() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"json::encode {1 2 3} {list num}"#).unwrap();
        assert_eq!(r.as_str(), "[1, 2, 3]");
    }

    #[test]
    fn test_encode_object_simple() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"json::encode {a 1 b hello} obj"#).unwrap();
        assert_eq!(r.as_str(), r#"{"a": "1", "b": "hello"}"#);
    }

    #[test]
    fn test_encode_object_typed() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"json::encode {a 1 b hello} {obj a num b str}"#).unwrap();
        assert_eq!(r.as_str(), r#"{"a": 1, "b": "hello"}"#);
    }

    #[test]
    fn test_encode_object_wildcard() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"json::encode {a 1 b 2 c hello} {obj c str * num}"#).unwrap();
        assert_eq!(r.as_str(), r#"{"a": 1, "b": 2, "c": "hello"}"#);
    }

    #[test]
    fn test_encode_mixed() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"json::encode {{a b c} 42} {mixed list num}"#).unwrap();
        assert_eq!(r.as_str(), r#"[["a", "b", "c"], 42]"#);
    }

    #[test]
    fn test_encode_string_escapes() {
        let mut interp = Interp::new();
        interp.eval(r#"set s "line1\nline2""#).unwrap();
        let r = interp.eval("json::encode $s str").unwrap();
        assert_eq!(r.as_str(), r#""line1\nline2""#);
    }

    #[test]
    fn test_encode_infinity() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("json::encode Inf num").unwrap().as_str(), "Infinity");
        assert_eq!(interp.eval("json::encode -Inf num").unwrap().as_str(), "-Infinity");
    }

    #[test]
    fn test_encode_null() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("json::encode null num").unwrap().as_str(), "null");
    }

    #[test]
    fn test_encode_booleans_list() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"json::encode {1 0 yes no true false} {list bool}"#).unwrap();
        assert_eq!(r.as_str(), "[true, false, true, false, true, false]");
    }

    // ── Roundtrip tests ────────────────────────────────────────

    #[test]
    fn test_roundtrip_object() {
        let mut interp = Interp::new();
        let d = decode(&mut interp, r#"{"name":"test","count":42}"#).unwrap();
        interp.set_var("d", d).unwrap();
        let encoded = interp.eval(r#"json::encode $d {obj count num name str}"#).unwrap();
        assert!(encoded.as_str().contains(r#""count": 42"#));
        assert!(encoded.as_str().contains(r#""name": "test""#));
    }

    #[test]
    fn test_roundtrip_with_schema() {
        let mut interp = Interp::new();
        let r = decode_opts(&mut interp, "-schema", r#"{"x":1,"y":"hello"}"#).unwrap();
        interp.set_var("result", r).unwrap();
        interp.eval("set data [lindex $result 0]").unwrap();
        interp.eval("set schema [lindex $result 1]").unwrap();
        let encoded = interp.eval("json::encode $data $schema").unwrap();
        assert!(encoded.as_str().contains(r#""x": 1"#));
        assert!(encoded.as_str().contains(r#""y": "hello""#));
    }

    // ── Ensemble syntax tests ──────────────────────────────────

    #[test]
    fn test_ensemble_decode() {
        let mut interp = Interp::new();
        let d = decode(&mut interp, r#"{"a":1}"#).unwrap();
        interp.set_var("d", d).unwrap();
        assert_eq!(interp.eval("dict get $d a").unwrap().as_str(), "1");
    }

    #[test]
    fn test_ensemble_encode() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"json encode "hello""#).unwrap();
        assert_eq!(r.as_str(), r#""hello""#);
    }

    // ── Surrogate pair test ────────────────────────────────────

    #[test]
    fn test_decode_surrogate_pair() {
        let mut interp = Interp::new();
        // U+1F600 (😀) = \uD83D\uDE00 in UTF-16
        let r = decode(&mut interp, r#"{"emoji":"\uD83D\uDE00"}"#).unwrap();
        interp.set_var("d", r).unwrap();
        assert_eq!(interp.eval("dict get $d emoji").unwrap().as_str(), "😀");
    }

    // ── Complex nested structure ───────────────────────────────

    #[test]
    fn test_decode_complex_nested() {
        let mut interp = Interp::new();
        let json = r#"{"users":[{"name":"Alice","scores":[95,87,92]},{"name":"Bob","scores":[78,88]}]}"#;
        let d = decode(&mut interp, json).unwrap();
        interp.set_var("d", d).unwrap();
        assert_eq!(
            interp.eval("dict get [lindex [dict get $d users] 0] name").unwrap().as_str(),
            "Alice"
        );
        assert_eq!(
            interp.eval("lindex [dict get [lindex [dict get $d users] 0] scores] 2").unwrap().as_str(),
            "92"
        );
    }
}
