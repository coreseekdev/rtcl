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
    Ok(result)
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

    /// Ternary `?:` — lowest precedence
    fn parse_ternary(&mut self) -> Result<Value> {
        let cond = self.parse_or()?;
        self.skip_whitespace();
        if self.match_op("?") {
            let then_val = self.parse_ternary()?;
            self.expect(":")?;
            let else_val = self.parse_ternary()?;
            if cond.is_true() {
                Ok(then_val)
            } else {
                Ok(else_val)
            }
        } else {
            Ok(cond)
        }
    }

    /// Logical OR `||`
    fn parse_or(&mut self) -> Result<Value> {
        let mut left = self.parse_and()?;
        while self.match_op("||") {
            if left.is_true() {
                // Short-circuit: skip parsing the RHS but consume the tokens
                self.skip_or_operand()?;
            } else {
                let right = self.parse_and()?;
                left = Value::from_bool(right.is_true());
            }
        }
        Ok(left)
    }

    /// Logical AND `&&` (short-circuit)
    fn parse_and(&mut self) -> Result<Value> {
        let mut left = self.parse_bitor()?;
        while self.match_op("&&") {
            if !left.is_true() {
                // Short-circuit: skip parsing the RHS but consume the tokens
                self.skip_and_operand()?;
            } else {
                let right = self.parse_bitor()?;
                left = Value::from_bool(right.is_true());
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
                left = match (left.as_float(), right.as_float()) {
                    (Some(a), Some(b)) => Value::from_bool((a - b).abs() < f64::EPSILON),
                    _ => Value::from_bool(left.as_str() == right.as_str()),
                };
            } else if self.match_op("!=") {
                let right = self.parse_relational()?;
                left = match (left.as_float(), right.as_float()) {
                    (Some(a), Some(b)) => Value::from_bool((a - b).abs() >= f64::EPSILON),
                    _ => Value::from_bool(left.as_str() != right.as_str()),
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
            left = match (left.as_float(), right.as_float()) {
                (Some(a), Some(b)) => Value::from_bool(match op {
                    "<" => a < b,
                    ">" => a > b,
                    "<=" => a <= b,
                    ">=" => a >= b,
                    _ => false,
                }),
                _ => Value::from_bool(match op {
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
                left = Value::from_int(a % b);
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
            match (base.as_float(), exp.as_float()) {
                (Some(a), Some(b)) => {
                    let result = a.powf(b);
                    if result.fract() == 0.0 && result.abs() < i64::MAX as f64 {
                        Ok(Value::from_int(result as i64))
                    } else {
                        Ok(Value::from_float(result))
                    }
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
            return Ok(Value::from_bool(!val.is_true()));
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
            if self.is_digit() || self.peek() == '(' || self.peek() == '$' || self.peek() == '[' || self.peek() == '.' {
                let val = self.parse_unary()?;
                return match val.as_float() {
                    Some(n) => {
                        if n.fract() == 0.0 && n.abs() < i64::MAX as f64 {
                            Ok(Value::from_int(-(n as i64)))
                        } else {
                            Ok(Value::from_float(-n))
                        }
                    }
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

        // Check for boolean literals
        match ident.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" => return Ok(Value::from_bool(true)),
            "false" | "no" | "off" => return Ok(Value::from_bool(false)),
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
            s.push(self.advance());
            if self.peek() == '+' || self.peek() == '-' {
                s.push(self.advance());
            }
            while self.is_digit() {
                s.push(self.advance());
            }
        }

        if s.contains('.') || s.contains('e') || s.contains('E') {
            Ok(Value::from_float(s.parse().unwrap_or(0.0)))
        } else {
            Ok(Value::from_int(s.parse().unwrap_or(0)))
        }
    }

    fn parse_string(&mut self) -> Result<Value> {
        let quote = self.advance();
        let mut s = String::new();
        let close = if quote == '{' { '}' } else { quote };
        let mut depth = if quote == '{' { 1i32 } else { 0 };

        while !self.is_at_end() {
            if quote == '{' {
                let c = self.peek();
                if c == '{' { depth += 1; }
                if c == '}' {
                    depth -= 1;
                    if depth == 0 { self.advance(); break; }
                }
                s.push(self.advance());
            } else {
                if self.peek() == close { self.advance(); break; }
                if self.peek() == '\\' {
                    self.advance();
                    if !self.is_at_end() {
                        s.push(self.parse_escape_char());
                    }
                } else {
                    s.push(self.advance());
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

    /// Match a word operator (requires non-alphanumeric boundary after it).
    fn match_word_op(&mut self, op: &str) -> bool {
        self.skip_whitespace();
        let op_chars: Vec<char> = op.chars().collect();
        let end = self.pos + op_chars.len();
        if end <= self.chars.len() {
            let slice: String = self.chars[self.pos..end].iter().collect();
            if slice == op {
                // Check boundary: next char must not be alphanumeric or '_'
                let next = self.chars.get(end).copied().unwrap_or('\0');
                if !next.is_alphanumeric() && next != '_' {
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

    /// Numeric binary operation, returning int when possible.
    fn numeric_binop(&self, left: &Value, right: &Value, op: char) -> Result<Value> {
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
                Ok(self.float_or_int(result))
            }
            _ => Err(Error::type_mismatch("number", "non-numeric value")),
        }
    }

    /// Return int if the float has no fractional part and fits in i64.
    fn float_or_int(&self, f: f64) -> Value {
        super::expr_funcs::float_or_int(f)
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
