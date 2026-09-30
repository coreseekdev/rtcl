//! Tcl expression evaluator
//!
//! Supports arithmetic, comparison, logical, bitwise, ternary, and string operations.
//! Operator precedence (lowest to highest):
//!   ternary `?:`, `||`, `&&`, `|`, `^`, `&`,
//!   `==` `!=` `eq` `ne` `in` `ni`, `<` `<=` `>` `>=`, `<<` `>>`,
//!   `+` `-`, `*` `/` `%`, `**`, unary `- + ! ~`

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;

/// Evaluate a Tcl expression
pub fn eval_expr(interp: &mut Interp, expr: &str) -> Result<Value> {
    let mut parser = ExprParser::new(expr, interp);
    let result = parser.parse_ternary()?;
    Ok(canonicalize_result(result))
}

/// Tcl: when the result of an expression is a string that looks like a
/// number, it is converted to the number's canonical form — `expr {$x}`
/// with x="1e15" yields `1000000000000000.0`, `" 1"` yields `1`, `0x10`
/// yields `16`. Non-numeric strings pass through unchanged. (Numeric
/// *operators* convert their operands individually; `eq`/`ne` keep
/// comparing the original strings.)
fn canonicalize_result(v: Value) -> Value {
    if v.type_name() != "string" {
        return v;
    }
    if let Some(i) = v.as_int() {
        return Value::from_int(i);
    }
    // A plain decimal integer beyond i64 is a Tcl bignum; keep its exact
    // text rather than degrading it to a rounded double (E4).
    let t = v.as_str().trim();
    let digits = t.strip_prefix(['-', '+']).unwrap_or(t);
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        return v;
    }
    if let Some(f) = v.as_float() {
        return super::expr_funcs::float_value(f);
    }
    v
}

/// Expression parser
struct ExprParser<'a> {
    chars: Vec<char>,
    pos: usize,
    interp: &'a mut Interp,
}

impl<'a> ExprParser<'a> {
    fn new(expr: &str, interp: &'a mut Interp) -> Self {
        ExprParser {
            chars: expr.chars().collect(),
            pos: 0,
            interp,
        }
    }

    /// Ternary `?:` — lowest precedence. Lazy: only the taken branch is
    /// evaluated; the other branch is skipped without side effects.
    fn parse_ternary(&mut self) -> Result<Value> {
        let cond = self.parse_or()?;
        self.skip_whitespace();
        if self.match_op("?") {
            if super::expr_funcs::strict_bool(&cond)? {
                let then_val = self.parse_ternary()?;
                self.expect(":")?;
                // Skip the untaken else-branch without evaluating it
                self.skip_ternary_operand()?;
                Ok(then_val)
            } else {
                // Skip the untaken then-branch up to the ':'
                self.skip_ternary_operand()?;
                self.expect(":")?;
                self.parse_ternary()
            }
        } else {
            Ok(cond)
        }
    }

    /// Logical OR `||`
    fn parse_or(&mut self) -> Result<Value> {
        let mut left = self.parse_and()?;
        while self.match_op("||") {
            if super::expr_funcs::strict_bool(&left)? {
                // Short-circuit: skip parsing the RHS but consume the tokens
                self.skip_or_operand()?;
                // Tcl: the result is always a boolean, never the operand.
                left = Value::from_bool(true);
            } else {
                let right = self.parse_and()?;
                left = Value::from_bool(super::expr_funcs::strict_bool(&right)?);
            }
        }
        Ok(left)
    }

    /// Logical AND `&&` (short-circuit)
    fn parse_and(&mut self) -> Result<Value> {
        let mut left = self.parse_bitor()?;
        while self.match_op("&&") {
            if !super::expr_funcs::strict_bool(&left)? {
                // Short-circuit: skip parsing the RHS but consume the tokens
                self.skip_and_operand()?;
                left = Value::from_bool(false);
            } else {
                let right = self.parse_bitor()?;
                left = Value::from_bool(super::expr_funcs::strict_bool(&right)?);
            }
        }
        Ok(left)
    }

    /// Bitwise OR `|`
    fn parse_bitor(&mut self) -> Result<Value> {
        let mut left = self.parse_bitxor()?;
        loop {
            self.skip_whitespace();
            // Match `|` but not `||`
            if self.peek() == '|' && self.peek_at(1) != '|' {
                self.advance();
                let right = self.parse_bitxor()?;
                let a = self.as_int_val(&left)?;
                let b = self.as_int_val(&right)?;
                left = Value::from_int(a | b);
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// Bitwise XOR `^`
    fn parse_bitxor(&mut self) -> Result<Value> {
        let mut left = self.parse_bitand()?;
        loop {
            self.skip_whitespace();
            if self.peek() == '^' {
                self.advance();
                let right = self.parse_bitand()?;
                let a = self.as_int_val(&left)?;
                let b = self.as_int_val(&right)?;
                left = Value::from_int(a ^ b);
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// Bitwise AND `&`
    fn parse_bitand(&mut self) -> Result<Value> {
        let mut left = self.parse_equality()?;
        loop {
            self.skip_whitespace();
            // Match `&` but not `&&`
            if self.peek() == '&' && self.peek_at(1) != '&' {
                self.advance();
                let right = self.parse_equality()?;
                let a = self.as_int_val(&left)?;
                let b = self.as_int_val(&right)?;
                left = Value::from_int(a & b);
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// Equality: `==`, `!=`, `eq`, `ne`, `in`, `ni`, `lt`, `gt`, `le`, `ge`, `=*`, `=~`
    fn parse_equality(&mut self) -> Result<Value> {
        let mut left = self.parse_relational()?;
        loop {
            if self.match_op("==") {
                let right = self.parse_relational()?;
                // NaN compares unequal to everything (expr-22.9).
                if super::expr_funcs::nan_pair(&left, &right) {
                    left = Value::from_bool(false);
                    continue;
                }
                // Exact numeric comparison when both operands are numeric
                // (int pairs compare as i64, no EPSILON tolerance);
                // string comparison otherwise.
                left = match super::expr_funcs::numeric_cmp(&left, &right) {
                    Some(ord) => Value::from_bool(ord == core::cmp::Ordering::Equal),
                    None => Value::from_bool(left.as_str() == right.as_str()),
                };
            } else if self.match_op("!=") {
                let right = self.parse_relational()?;
                if super::expr_funcs::nan_pair(&left, &right) {
                    left = Value::from_bool(true);
                    continue;
                }
                left = match super::expr_funcs::numeric_cmp(&left, &right) {
                    Some(ord) => Value::from_bool(ord != core::cmp::Ordering::Equal),
                    None => Value::from_bool(left.as_str() != right.as_str()),
                };
            } else if self.match_op("=*") {
                // Glob match: left =* pattern
                let right = self.parse_relational()?;
                left = Value::from_bool(
                    crate::interp::glob_match(right.as_str(), left.as_str()),
                );
            } else if self.match_op("=~") {
                // Regexp match: left =~ pattern
                let right = self.parse_relational()?;
                #[cfg(feature = "regexp")]
                {
                    let matched = regex::Regex::new(right.as_str())
                        .map(|re| re.is_match(left.as_str()))
                        .unwrap_or(false);
                    left = Value::from_bool(matched);
                }
                #[cfg(not(feature = "regexp"))]
                {
                    let _ = right;
                    return Err(Error::runtime(
                        "=~ operator requires 'regexp' feature",
                        crate::error::ErrorCode::InvalidOp,
                    ));
                }
            } else if self.match_word_op("eq") {
                let right = self.parse_relational()?;
                left = Value::from_bool(left.as_str() == right.as_str());
            } else if self.match_word_op("ne") {
                let right = self.parse_relational()?;
                left = Value::from_bool(left.as_str() != right.as_str());
            } else if self.match_word_op("in") {
                let right = self.parse_relational()?;
                let items = right.as_list().unwrap_or_default();
                let found = items.iter().any(|v| v.as_str() == left.as_str());
                left = Value::from_bool(found);
            } else if self.match_word_op("ni") {
                let right = self.parse_relational()?;
                let items = right.as_list().unwrap_or_default();
                let found = items.iter().any(|v| v.as_str() == left.as_str());
                left = Value::from_bool(!found);
            } else if self.match_word_op("lt") {
                let right = self.parse_relational()?;
                left = Value::from_bool(left.as_str() < right.as_str());
            } else if self.match_word_op("gt") {
                let right = self.parse_relational()?;
                left = Value::from_bool(left.as_str() > right.as_str());
            } else if self.match_word_op("le") {
                let right = self.parse_relational()?;
                left = Value::from_bool(left.as_str() <= right.as_str());
            } else if self.match_word_op("ge") {
                let right = self.parse_relational()?;
                left = Value::from_bool(left.as_str() >= right.as_str());
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// Relational: `<`, `<=`, `>`, `>=`
    fn parse_relational(&mut self) -> Result<Value> {
        let mut left = self.parse_shift()?;
        loop {
            let op = if self.match_op("<=") {
                "<="
            } else if self.match_op(">=") {
                ">="
            } else if self.match_op("<") {
                "<"
            } else if self.match_op(">") {
                ">"
            } else {
                break;
            };
            let right = self.parse_shift()?;
            use core::cmp::Ordering;
            // NaN: every relational comparison is false.
            if super::expr_funcs::nan_pair(&left, &right) {
                left = Value::from_bool(false);
                continue;
            }
            left = match super::expr_funcs::numeric_cmp(&left, &right) {
                Some(ord) => Value::from_bool(match op {
                    "<" => ord == Ordering::Less,
                    ">" => ord == Ordering::Greater,
                    "<=" => ord != Ordering::Greater,
                    ">=" => ord != Ordering::Less,
                    _ => false,
                }),
                None => Value::from_bool(match op {
                    "<" => left.as_str() < right.as_str(),
                    ">" => left.as_str() > right.as_str(),
                    "<=" => left.as_str() <= right.as_str(),
                    ">=" => left.as_str() >= right.as_str(),
                    _ => false,
                }),
            };
        }
        Ok(left)
    }

    /// Shift: `<<`, `>>`, `<<<`, `>>>`
    fn parse_shift(&mut self) -> Result<Value> {
        let mut left = self.parse_additive()?;
        loop {
            if self.match_op("<<<") {
                let right = self.parse_additive()?;
                let a = self.as_int_val(&left)? as u64;
                let b = self.as_int_val(&right)? as u32;
                let bits = 64u32;
                let shift = b % bits;
                let rotated = if shift == 0 { a } else { (a << shift) | (a >> (bits - shift)) };
                left = Value::from_int(rotated as i64);
            } else if self.match_op("<<") {
                let right = self.parse_additive()?;
                let a = self.as_int_val(&left)?;
                let b = self.as_int_val(&right)?;
                left = Value::from_int(a << (b & 63));
            } else if self.match_op(">>>") {
                let right = self.parse_additive()?;
                let a = self.as_int_val(&left)? as u64;
                let b = self.as_int_val(&right)? as u32;
                let bits = 64u32;
                let shift = b % bits;
                let rotated = if shift == 0 { a } else { (a >> shift) | (a << (bits - shift)) };
                left = Value::from_int(rotated as i64);
            } else if self.match_op(">>") {
                let right = self.parse_additive()?;
                let a = self.as_int_val(&left)?;
                let b = self.as_int_val(&right)?;
                left = Value::from_int(a >> (b & 63));
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// Additive: `+`, `-`
    fn parse_additive(&mut self) -> Result<Value> {
        let mut left = self.parse_multiplicative()?;
        loop {
            self.skip_whitespace();
            if self.peek() == '+' {
                self.advance();
                let right = self.parse_multiplicative()?;
                left = self.numeric_binop(&left, &right, '+' )?;
            } else if self.peek() == '-' {
                // Distinguish unary minus from binary minus.
                // Binary minus: there must have been a value on the left.
                self.advance();
                let right = self.parse_multiplicative()?;
                left = self.numeric_binop(&left, &right, '-')?;
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// Multiplicative: `*`, `/`, `%`
    fn parse_multiplicative(&mut self) -> Result<Value> {
        let mut left = self.parse_power()?;
        loop {
            self.skip_whitespace();
            if self.peek() == '*' && self.peek_at(1) != '*' {
                self.advance();
                let right = self.parse_power()?;
                left = self.numeric_binop(&left, &right, '*')?;
            } else if self.peek() == '/' {
                self.advance();
                let right = self.parse_power()?;
                if let (Some(_), Some(0.0)) = (left.as_float(), right.as_float()) {
                    return Err(Error::DivisionByZero);
                }
                left = self.numeric_binop(&left, &right, '/')?;
            } else if self.peek() == '%' {
                self.advance();
                let right = self.parse_power()?;
                let a = self.as_int_val(&left)?;
                let b = self.as_int_val(&right)?;
                if b == 0 { return Err(Error::DivisionByZero); }
                // i64::MIN % -1 would overflow in Rust; the result is 0
                left = Value::from_int(if a == i64::MIN && b == -1 { 0 } else { floor_mod(a, b) });
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// Power: `**` (right-associative)
    fn parse_power(&mut self) -> Result<Value> {
        let base = self.parse_unary()?;
        if self.match_op("**") {
            let exp = self.parse_power()?; // right-associative: recurse
            // Tcl: two integer operands use integer exponentiation.
            if let (Some(a), Some(b)) = (base.as_int(), exp.as_int()) {
                return match a {
                    // Units short-circuit before any range check — tclsh
                    // computes 1**268435456 and (-1)**268435456 instantly.
                    0 if b < 0 => Err(Error::Msg(
                        "exponentiation of zero by negative power".to_string(),
                    )),
                    0 => Ok(Value::from_int(0)),
                    1 => Ok(Value::from_int(1)),
                    -1 => Ok(Value::from_int(if b % 2 == 0 { 1 } else { -1 })),
                    // Integer base with negative exponent yields 0
                    // (tclsh: `expr {2**-1}` → 0).
                    _ if b < 0 => Ok(Value::from_int(0)),
                    // tclsh refuses to build numbers of ≥ 2**28 bits
                    // (expr-23.54.12: 3**268435456 → "exponent too large").
                    _ if b >= 1 << 28 => Err(Error::Msg("exponent too large".to_string())),
                    _ => match a.checked_pow(b as u32) {
                        Some(r) => Ok(Value::from_int(r)),
                        // Overflow: Tcl widens to bignum; we promote to
                        // double instead (E4).
                        None => {
                            Ok(super::expr_funcs::float_value((a as f64).powf(b as f64)))
                        }
                    },
                };
            }
            match (base.as_float(), exp.as_float()) {
                (Some(a), Some(b)) => {
                    if a == 0.0 && b < 0.0 {
                        // 0.0**-1 / 0**-1.0 error like the integer form.
                        return Err(Error::Msg(
                            "exponentiation of zero by negative power".to_string(),
                        ));
                    }
                    Ok(super::expr_funcs::float_value(a.powf(b)))
                }
                _ => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        } else {
            Ok(base)
        }
    }

    /// Unary: `!`, `-`, `+`, `~`
    fn parse_unary(&mut self) -> Result<Value> {
        self.skip_whitespace();
        if self.match_op("!") {
            let val = self.parse_unary()?;
            return Ok(Value::from_bool(!super::expr_funcs::not_operand(&val)?));
        }
        if self.peek() == '~' {
            self.advance();
            let val = self.parse_unary()?;
            let n = self.as_int_val(&val)?;
            return Ok(Value::from_int(!n));
        }
        if self.peek() == '-' && !self.is_at_end() {
            // Only unary minus if we're at the start of unary context
            // (The additive parser handles binary minus)
            let saved = self.pos;
            self.advance();
            // Check if next char can start an expression
            self.skip_whitespace();
            // Alphabetic covers numeric identifiers like `inf` (`expr -inf`);
            // +/- chain into further unary signs (`expr --5` -> 5).
            if self.is_digit()
                || self.peek() == '('
                || self.peek() == '$'
                || self.peek() == '['
                || self.peek() == '.'
                || self.peek() == '-'
                || self.peek() == '+'
                || self.peek().is_ascii_alphabetic()
            {
                // tclsh: -9223372036854775808 is a valid integer literal
                // (the digits alone overflow, the sign rescues them).
                if self.chars[self.pos..].iter().take(19).collect::<String>() == "9223372036854775808"
                    && !self
                        .chars
                        .get(self.pos + 19)
                        .map(|c| c.is_ascii_digit() || *c == '.')
                        .unwrap_or(false)
                {
                    self.pos += 19;
                    return Ok(Value::from_int(i64::MIN));
                }
                let val = self.parse_unary()?;
                // Integer operands stay integers (-2 -> -2); float operands
                // stay floats (-2.0 -> -2.0).
                if let Some(i) = val.as_int() {
                    return Ok(match i.checked_neg() {
                        Some(r) => Value::from_int(r),
                        None => super::expr_funcs::float_value(-(i as f64)),
                    });
                }
                return match val.as_float() {
                    Some(n) => Ok(super::expr_funcs::float_value(-n)),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                };
            }
            // Not unary minus, restore
            self.pos = saved;
        }
        if self.match_op("+") {
            return self.parse_unary();
        }
        self.parse_primary()
    }

    /// Parse primary expression (literals, variables, function calls)
    fn parse_primary(&mut self) -> Result<Value> {
        self.skip_whitespace();

        // Parenthesized expression
        if self.match_op("(") {
            let val = self.parse_ternary()?;
            self.expect(")")?;
            return Ok(val);
        }

        // Command substitution
        if self.peek() == '[' {
            self.advance();
            let cmd = self.collect_bracket_command();
            return self.interp.eval(&cmd);
        }

        // Variable reference
        if self.peek() == '$' {
            self.advance();
            if self.peek() == '{' {
                // ${varname}
                self.advance();
                let mut name = String::new();
                while !self.is_at_end() && self.peek() != '}' {
                    name.push(self.advance());
                }
                if !self.is_at_end() { self.advance(); } // consume '}'
                return self.interp.get_var(&name).cloned();
            }
            let name = self.parse_var_name();
            return self.interp.get_var(&name).cloned();
        }

        // String literal
        if self.peek() == '"' || self.peek() == '{' {
            return self.parse_string();
        }

        // Number or function call
        if self.is_digit() || self.peek() == '.' {
            return self.parse_number();
        }

        // Identifier (could be boolean, function, or variable)
        let ident = self.parse_identifier();
        if ident.is_empty() {
            return Ok(Value::empty());
        }

        // Check for function call
        self.skip_whitespace();
        if self.peek() == '(' {
            return self.parse_function_call(&ident);
        }

        // Boolean literals keep their string form (`expr false` renders
        // "false", expr-21.1); operators coerce via strict_bool.
        match ident.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "false" | "no" | "off" => {
                return Ok(Value::from_str(&ident))
            }
            _ => {}
        }

        // Try as variable
        if self.interp.var_exists(&ident) {
            return self.interp.get_var(&ident).cloned();
        }

        // Try as number
        if let Ok(n) = ident.parse::<i64>() {
            return Ok(Value::from_int(n));
        }
        if let Ok(n) = ident.parse::<f64>() {
            if n.is_nan() {
                // tclsh: a bare `nan` operand is a domain error — NaN is
                // only reachable via the nan() function.
                return Err(super::expr_funcs::domain_error());
            }
            return Ok(Value::from_float(n));
        }

        // Return as string
        Ok(Value::from_str(&ident))
    }

    // -- Number / string / identifier parsing --------------------------------

    fn parse_number(&mut self) -> Result<Value> {
        let mut s = String::new();

        // Handle hex / binary / octal
        if self.peek() == '0' && self.pos + 1 < self.chars.len() {
            let next = self.chars[self.pos + 1];
            if next == 'x' || next == 'X' {
                s.push(self.advance());
                s.push(self.advance());
                while self.is_hex_digit() {
                    s.push(self.advance());
                }
                return Ok(Value::from_int(i64::from_str_radix(&s[2..], 16).unwrap_or(0)));
            }
            if next == 'b' || next == 'B' {
                s.push(self.advance());
                s.push(self.advance());
                while self.peek() == '0' || self.peek() == '1' {
                    s.push(self.advance());
                }
                return Ok(Value::from_int(i64::from_str_radix(&s[2..], 2).unwrap_or(0)));
            }
            if next == 'o' || next == 'O' {
                s.push(self.advance());
                s.push(self.advance());
                while self.peek() >= '0' && self.peek() <= '7' {
                    s.push(self.advance());
                }
                return Ok(Value::from_int(i64::from_str_radix(&s[2..], 8).unwrap_or(0)));
            }
        }

        while self.is_digit() {
            s.push(self.advance());
        }
        if self.peek() == '.' {
            s.push(self.advance());
            while self.is_digit() {
                s.push(self.advance());
            }
        }
        if self.peek() == 'e' || self.peek() == 'E' {
            // Lookahead: `e` only belongs to the number when digits (or a
            // sign then digits) follow — `3eq2` must stop at `3`.
            let save = self.pos;
            self.advance();
            let mut exp = String::new();
            exp.push(self.chars[self.pos - 1]);
            if self.peek() == '+' || self.peek() == '-' {
                exp.push(self.advance());
            }
            if self.is_digit() {
                s.push_str(&exp);
                while self.is_digit() {
                    s.push(self.advance());
                }
            } else {
                self.pos = save;
            }
        }

        self.check_number_trail(&s)?;

        if s.contains('.') || s.contains('e') || s.contains('E') {
            Ok(super::expr_funcs::float_value(s.parse().unwrap_or(0.0)))
        } else {
            match s.parse::<i64>() {
                Ok(n) => Ok(Value::from_int(n)),
                // Tcl widens oversized integer literals to bignum; we
                // promote to double instead (E4).
                Err(_) => Ok(super::expr_funcs::float_value(
                    s.parse().unwrap_or(f64::INFINITY),
                )),
            }
        }
    }

    /// tclsh: letters glued to a number are only legal as the word
    /// operators `eq`/`ne`/`in` followed by a digit-led operand
    /// (`3eq2`); any other digit-led alnum run is an invalid bareword
    /// (`3x2`, `3e`, `5mod3`, `3lt2`).  Rewinds to just after the digits
    /// so operator matching sees `eq`.
    fn check_number_trail(&mut self, prefix: &str) -> Result<()> {
        if !self.peek().is_ascii_alphabetic() {
            return Ok(());
        }
        let run_start = self.pos;
        while self.peek().is_ascii_alphanumeric() || self.peek() == '_' {
            self.advance();
        }
        let run: String = self.chars[run_start..self.pos].iter().collect();
        let word_op = run == "eq" || run == "ne" || run == "in";
        let glued = (run.starts_with("eq") || run.starts_with("ne") || run.starts_with("in"))
            && run
                .as_bytes()
                .get(2)
                .map(|b| b.is_ascii_digit() || *b == b'.')
                .unwrap_or(false);
        if word_op || glued {
            self.pos = run_start;
            return Ok(());
        }
        let whole: String = self.chars.iter().collect();
        let bare = format!("{}{}", prefix, run);
        Err(Error::runtime(
            format!(
                "invalid bareword \"{bw}\"\nin expression \"{w}\";\nshould be \"${bw}\" or \"{{{bw}}}\" or \"{bw}(...)\" or ...",
                bw = bare,
                w = whole
            ),
            crate::error::ErrorCode::Generic,
        ))
    }

    /// Collect the body of a `[...]` command substitution (the leading `[`
    /// is already consumed). Returns the script between balanced brackets.
    fn collect_bracket_command(&mut self) -> String {
        let mut cmd = String::new();
        let mut depth = 1;
        while !self.is_at_end() && depth > 0 {
            let c = self.advance();
            if c == '[' {
                depth += 1;
                cmd.push(c);
            } else if c == ']' {
                depth -= 1;
                if depth > 0 {
                    cmd.push(c);
                }
            } else {
                cmd.push(c);
            }
        }
        cmd
    }

    fn parse_string(&mut self) -> Result<Value> {
        let quote = self.advance();
        let mut s = String::new();
        let close = if quote == '{' { '}' } else { quote };
        let mut depth = if quote == '{' { 1i32 } else { 0 };

        while !self.is_at_end() {
            if quote == '{' {
                let c = self.peek();
                if c == '\\' {
                    // Backslash sequences stay literal inside braces, but an
                    // escaped brace does not participate in balancing.
                    s.push(self.advance());
                    if !self.is_at_end() {
                        s.push(self.advance());
                    }
                    continue;
                }
                if c == '{' { depth += 1; }
                if c == '}' {
                    depth -= 1;
                    if depth == 0 { self.advance(); break; }
                }
                s.push(self.advance());
            } else {
                // Double-quoted word: perform $ / [] / backslash substitution
                match self.peek() {
                    c if c == close => { self.advance(); break; }
                    '\\' => {
                        self.advance();
                        if !self.is_at_end() {
                            s.push(self.parse_escape_char());
                        }
                    }
                    '$' => {
                        self.advance();
                        let name = if self.peek() == '{' {
                            // ${varname}
                            self.advance();
                            let mut name = String::new();
                            while !self.is_at_end() && self.peek() != '}' {
                                name.push(self.advance());
                            }
                            if !self.is_at_end() { self.advance(); } // consume '}'
                            name
                        } else {
                            self.parse_var_name()
                        };
                        if name.is_empty() {
                            // "$" with no variable name is a literal dollar sign
                            s.push('$');
                        } else {
                            let val = self.interp.get_var(&name).cloned()?;
                            s.push_str(val.as_str());
                        }
                    }
                    '[' => {
                        self.advance();
                        let cmd = self.collect_bracket_command();
                        let val = self.interp.eval(&cmd)?;
                        s.push_str(val.as_str());
                    }
                    _ => {
                        s.push(self.advance());
                    }
                }
            }
        }

        Ok(Value::from_str(&s))
    }

    fn parse_escape_char(&mut self) -> char {
        match self.advance() {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            '\\' => '\\',
            '"' => '"',
            c => c,
        }
    }

    fn parse_identifier(&mut self) -> String {
        let mut s = String::new();
        while !self.is_at_end() {
            let c = self.peek();
            if c.is_alphanumeric() || c == '_' {
                s.push(self.advance());
            } else {
                break;
            }
        }
        s
    }

    /// Parse a variable name after `$`.
    fn parse_var_name(&mut self) -> String {
        let mut s = String::new();
        while !self.is_at_end() {
            let c = self.peek();
            if c.is_alphanumeric() || c == '_' || c == ':' {
                s.push(self.advance());
            } else if c == '(' {
                // Array reference: name(index)
                s.push(self.advance());
                while !self.is_at_end() && self.peek() != ')' {
                    s.push(self.advance());
                }
                if !self.is_at_end() {
                    s.push(self.advance()); // consume ')'
                }
                break;
            } else {
                break;
            }
        }
        s
    }

    // -- Function calls ------------------------------------------------------

    fn parse_function_call(&mut self, name: &str) -> Result<Value> {
        self.expect("(")?;
        let mut args = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek() == ')' { break; }
            let arg = self.parse_ternary()?;
            args.push(arg);
            self.skip_whitespace();
            if self.peek() == ',' { self.advance(); } else { break; }
        }
        self.expect(")")?;

        let rand_seed = (self.interp as *const Interp as usize) ^ self.pos;
        super::expr_funcs::call_math_func(name, args, rand_seed)
    }

    // -- Helpers -------------------------------------------------------------

    fn peek(&self) -> char {
        self.chars.get(self.pos).copied().unwrap_or('\0')
    }

    fn peek_at(&self, offset: usize) -> char {
        self.chars.get(self.pos + offset).copied().unwrap_or('\0')
    }

    fn advance(&mut self) -> char {
        let c = self.peek();
        self.pos += 1;
        c
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn skip_whitespace(&mut self) {
        while !self.is_at_end() && self.peek().is_whitespace() {
            self.advance();
        }
    }

    /// Match a symbol operator (does not check word boundaries).
    fn match_op(&mut self, op: &str) -> bool {
        self.skip_whitespace();
        let op_chars: Vec<char> = op.chars().collect();
        if self.pos + op_chars.len() <= self.chars.len() {
            let slice: String = self.chars[self.pos..self.pos + op_chars.len()].iter().collect();
            if slice == op {
                self.pos += op_chars.len();
                return true;
            }
        }
        false
    }

    /// Match a word operator (requires non-alphanumeric boundary after
    /// it).  Exception: a number immediately followed by eq/ne/in
    /// (`3eq2`, validated by `check_number_trail`) still lexes as
    /// operator + digit-led operand, tclsh-style.
    fn match_word_op(&mut self, op: &str) -> bool {
        self.skip_whitespace();
        let op_chars: Vec<char> = op.chars().collect();
        let end = self.pos + op_chars.len();
        if end <= self.chars.len() {
            let slice: String = self.chars[self.pos..end].iter().collect();
            if slice == op {
                let prev = if self.pos > 0 {
                    self.chars[self.pos - 1]
                } else {
                    '\0'
                };
                // Check boundary: next char must not be alphanumeric or '_',
                // unless we are glued to a preceding digit (`3eq2`).
                let next = self.chars.get(end).copied().unwrap_or('\0');
                if (!next.is_alphanumeric() && next != '_') || prev.is_ascii_digit() {
                    self.pos = end;
                    return true;
                }
            }
        }
        false
    }

    fn expect(&mut self, s: &str) -> Result<()> {
        if !self.match_op(s) {
            Err(Error::syntax(
                format!("expected '{}'", s),
                0,
                self.pos,
            ))
        } else {
            Ok(())
        }
    }

    fn is_digit(&self) -> bool {
        self.peek().is_ascii_digit()
    }

    fn is_hex_digit(&self) -> bool {
        let c = self.peek();
        c.is_ascii_digit() || ('a'..='f').contains(&c) || ('A'..='F').contains(&c)
    }

    /// Convert a Value to i64, returning an error if not numeric.
    fn as_int_val(&self, v: &Value) -> Result<i64> {
        v.as_int().or_else(|| v.as_float().map(|f| f as i64))
            .ok_or_else(|| Error::type_mismatch("integer", v.as_str()))
    }

    /// Numeric binary operation. Integer operands use integer arithmetic
    /// (Tcl semantics: `/` is floor division, result is int); on i64
    /// overflow the operation is promoted to double (Tcl widens to bignum;
    /// rtcl has no bignum — see E4 in specs/DIVERGENCES.md). If either side
    /// is float, compute in f64 and the result stays a float.
    fn numeric_binop(&self, left: &Value, right: &Value, op: char) -> Result<Value> {
        if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
            let promoted = |f: f64| super::expr_funcs::float_value(f);
            return match op {
                '+' => Ok(match a.checked_add(b) {
                    Some(r) => Value::from_int(r),
                    None => promoted(a as f64 + b as f64),
                }),
                '-' => Ok(match a.checked_sub(b) {
                    Some(r) => Value::from_int(r),
                    None => promoted(a as f64 - b as f64),
                }),
                '*' => Ok(match a.checked_mul(b) {
                    Some(r) => Value::from_int(r),
                    None => promoted(a as f64 * b as f64),
                }),
                '/' => {
                    if b == 0 { return Err(Error::DivisionByZero); }
                    // i64::MIN / -1 overflows in Rust; Tcl widens to bignum
                    if a == i64::MIN && b == -1 {
                        return Ok(promoted(-(a as f64)));
                    }
                    Ok(Value::from_int(floor_div(a, b)))
                }
                _ => Err(Error::runtime("unknown op", crate::error::ErrorCode::InvalidOp)),
            };
        }
        match (left.as_float(), right.as_float()) {
            (Some(a), Some(b)) => {
                let result = match op {
                    '+' => a + b,
                    '-' => a - b,
                    '*' => a * b,
                    '/' => {
                        if b == 0.0 { return Err(Error::DivisionByZero); }
                        a / b
                    }
                    _ => return Err(Error::runtime("unknown op", crate::error::ErrorCode::InvalidOp)),
                };
                Ok(super::expr_funcs::float_value(result))
            }
            _ => Err(Error::type_mismatch("number", "non-numeric value")),
        }
    }
    /// Skip one `parse_and` level operand without evaluating.
    /// Used for `||` short-circuit when LHS is true.
    /// Stops at: end, `||` at depth 0, `?` at depth 0.
    fn skip_or_operand(&mut self) -> Result<()> {
        self.skip_balanced(&["||", "?"])
    }

    /// Skip one `parse_bitor` level operand without evaluating.
    /// Used for `&&` short-circuit when LHS is false.
    /// Stops at: end, `&&` at depth 0, `||` at depth 0, `?` at depth 0.
    fn skip_and_operand(&mut self) -> Result<()> {
        self.skip_balanced(&["&&", "||", "?"])
    }

    /// Skip one ternary-level operand without evaluating it.
    /// Used for `?:` lazy evaluation of the untaken branch.
    /// Stops (without consuming) at: a top-level `:` (the then/else
    /// separator of this or an enclosing ternary), `)` or `,` at depth 0
    /// (enclosing paren group or function argument list), or end of input.
    /// Nested `?...:` pairs encountered while skipping are tracked so their
    /// `:` is not mistaken for the enclosing separator. A `:` that is part
    /// of a `::` namespace qualifier is never a stop token.
    fn skip_ternary_operand(&mut self) -> Result<()> {
        let mut depth: i32 = 0;         // parenthesis depth
        let mut ternary_depth: i32 = 0; // nested ?: opened while skipping

        loop {
            self.skip_whitespace();
            if self.is_at_end() {
                break;
            }

            if depth == 0 {
                match self.peek() {
                    '?' => {
                        self.advance();
                        ternary_depth += 1;
                        continue;
                    }
                    ':' if !self.is_scoped_colon() => {
                        if ternary_depth > 0 {
                            self.advance();
                            ternary_depth -= 1;
                            continue;
                        }
                        // A top-level ':' closes an outer ternary branch
                        // (then-separator or the else of an enclosing `?:`);
                        // leave it for the caller's expect(":").
                        return Ok(());
                    }
                    ')' | ',' => return Ok(()),
                    _ => {}
                }
            }

            let c = self.peek();
            match c {
                '(' => { self.advance(); depth += 1; }
                ')' => {
                    if depth <= 0 {
                        break; // unmatched — let caller handle
                    }
                    self.advance();
                    depth -= 1;
                }
                '[' => {
                    // Command substitution — skip balanced brackets
                    self.advance();
                    let mut bdepth = 1;
                    while !self.is_at_end() && bdepth > 0 {
                        match self.advance() {
                            '[' => bdepth += 1,
                            ']' => bdepth -= 1,
                            '\\' => { self.advance(); } // skip escaped char
                            _ => {}
                        }
                    }
                }
                '"' => {
                    // String literal — skip to closing quote
                    self.advance();
                    while !self.is_at_end() && self.peek() != '"' {
                        if self.peek() == '\\' {
                            self.advance(); // skip escape char
                        }
                        self.advance();
                    }
                    if !self.is_at_end() {
                        self.advance(); // closing quote
                    }
                }
                '{' => {
                    // Braced string — skip balanced braces
                    self.advance();
                    let mut bdepth = 1;
                    while !self.is_at_end() && bdepth > 0 {
                        match self.advance() {
                            '{' => bdepth += 1,
                            '}' => bdepth -= 1,
                            '\\' => { self.advance(); }
                            _ => {}
                        }
                    }
                }
                _ => { self.advance(); }
            }
        }

        Ok(())
    }

    /// True if the `:` at the cursor is part of a `::` namespace qualifier
    /// (e.g. inside `$::ns::var` or `::ns::func(...)`) rather than the
    /// ternary else separator.
    fn is_scoped_colon(&self) -> bool {
        self.peek_at(1) == ':' || (self.pos > 0 && self.chars[self.pos - 1] == ':')
    }

    /// Consume characters, respecting balanced delimiters, until we reach
    /// one of the `stop_ops` at nesting depth 0 or end of input.
    /// Does NOT consume the stop operator itself.
    fn skip_balanced(&mut self, stop_ops: &[&str]) -> Result<()> {
        let mut depth: i32 = 0; // parenthesis depth

        loop {
            self.skip_whitespace();
            if self.is_at_end() {
                break;
            }

            // Check for stop operators at depth 0
            if depth == 0 {
                for op in stop_ops {
                    let op_len = op.len();
                    if self.pos + op_len <= self.chars.len() {
                        let slice: String = self.chars[self.pos..self.pos + op_len].iter().collect();
                        if slice == *op {
                            return Ok(()); // don't consume the stop op
                        }
                    }
                }
            }

            let c = self.peek();
            match c {
                '(' => { self.advance(); depth += 1; }
                ')' => {
                    if depth <= 0 {
                        break; // unmatched — let caller handle
                    }
                    self.advance();
                    depth -= 1;
                }
                '[' => {
                    // Command substitution — skip balanced brackets
                    self.advance();
                    let mut bdepth = 1;
                    while !self.is_at_end() && bdepth > 0 {
                        match self.advance() {
                            '[' => bdepth += 1,
                            ']' => bdepth -= 1,
                            '\\' => { self.advance(); } // skip escaped char
                            _ => {}
                        }
                    }
                }
                '"' => {
                    // String literal — skip to closing quote
                    self.advance();
                    while !self.is_at_end() && self.peek() != '"' {
                        if self.peek() == '\\' {
                            self.advance(); // skip escape char
                        }
                        self.advance();
                    }
                    if !self.is_at_end() {
                        self.advance(); // closing quote
                    }
                }
                '{' => {
                    // Braced string — skip balanced braces
                    self.advance();
                    let mut bdepth = 1;
                    while !self.is_at_end() && bdepth > 0 {
                        match self.advance() {
                            '{' => bdepth += 1,
                            '}' => bdepth -= 1,
                            '\\' => { self.advance(); }
                            _ => {}
                        }
                    }
                }
                _ => { self.advance(); }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "expr_tests.rs"]
mod tests;

/// Tcl integer division: floor semantics (rounds toward negative infinity),
/// so `-10 / 3 == -4`. Precondition: `b != 0`.
pub(crate) fn floor_div(a: i64, b: i64) -> i64 {
    let q = a / b;
    if (a % b != 0) && ((a < 0) != (b < 0)) { q - 1 } else { q }
}

/// Tcl integer modulo: result takes the sign of the divisor,
/// so `-10 % 3 == 2` and `10 % -3 == -2`. Precondition: `b != 0`.
pub(crate) fn floor_mod(a: i64, b: i64) -> i64 {
    let r = a % b;
    if r != 0 && ((r < 0) != (b < 0)) { r + b } else { r }
}
