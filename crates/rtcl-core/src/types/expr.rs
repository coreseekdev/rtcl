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

    /// Equality: `==`, `!=`, `eq`, `ne`, `in`, `ni`
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

    /// Shift: `<<`, `>>`
    fn parse_shift(&mut self) -> Result<Value> {
        let mut left = self.parse_additive()?;
        loop {
            if self.match_op("<<") {
                let right = self.parse_additive()?;
                let a = self.as_int_val(&left)?;
                let b = self.as_int_val(&right)?;
                left = Value::from_int(a << (b & 63));
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
                match (left.as_float(), right.as_float()) {
                    (Some(_), Some(b)) if b == 0.0 => return Err(Error::DivisionByZero),
                    _ => {}
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

        match name {
            "abs" => {
                self.require_args(name, 1, args.len())?;
                match args[0].as_float() {
                    Some(n) => Ok(self.float_or_int(n.abs())),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "int" | "entier" => {
                self.require_args(name, 1, args.len())?;
                match args[0].as_float() {
                    Some(n) => Ok(Value::from_int(n as i64)),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "wide" => {
                self.require_args(name, 1, args.len())?;
                match args[0].as_float() {
                    Some(n) => Ok(Value::from_int(n as i64)),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "double" => {
                self.require_args(name, 1, args.len())?;
                match args[0].as_float() {
                    Some(n) => Ok(Value::from_float(n)),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "bool" => {
                self.require_args(name, 1, args.len())?;
                Ok(Value::from_bool(args[0].is_true()))
            }
            "round" => {
                self.require_args(name, 1, args.len())?;
                match args[0].as_float() {
                    Some(n) => Ok(Value::from_int(n.round() as i64)),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "floor" => {
                self.require_args(name, 1, args.len())?;
                match args[0].as_float() {
                    Some(n) => Ok(Value::from_int(n.floor() as i64)),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "ceil" => {
                self.require_args(name, 1, args.len())?;
                match args[0].as_float() {
                    Some(n) => Ok(Value::from_int(n.ceil() as i64)),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "sqrt" => {
                self.require_args(name, 1, args.len())?;
                match args[0].as_float() {
                    Some(n) => Ok(Value::from_float(n.sqrt())),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "pow" => {
                self.require_args(name, 2, args.len())?;
                match (args[0].as_float(), args[1].as_float()) {
                    (Some(a), Some(b)) => Ok(self.float_or_int(a.powf(b))),
                    _ => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "fmod" => {
                self.require_args(name, 2, args.len())?;
                match (args[0].as_float(), args[1].as_float()) {
                    (Some(a), Some(b)) => Ok(Value::from_float(a % b)),
                    _ => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "atan2" => {
                self.require_args(name, 2, args.len())?;
                match (args[0].as_float(), args[1].as_float()) {
                    (Some(a), Some(b)) => Ok(Value::from_float(a.atan2(b))),
                    _ => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "hypot" => {
                self.require_args(name, 2, args.len())?;
                match (args[0].as_float(), args[1].as_float()) {
                    (Some(a), Some(b)) => Ok(Value::from_float(a.hypot(b))),
                    _ => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "log" | "log10" | "exp"
            | "sinh" | "cosh" | "tanh" => {
                self.require_args(name, 1, args.len())?;
                match args[0].as_float() {
                    Some(n) => {
                        let result = match name {
                            "sin" => n.sin(),
                            "cos" => n.cos(),
                            "tan" => n.tan(),
                            "asin" => n.asin(),
                            "acos" => n.acos(),
                            "atan" => n.atan(),
                            "log" => n.ln(),
                            "log10" => n.log10(),
                            "exp" => n.exp(),
                            "sinh" => n.sinh(),
                            "cosh" => n.cosh(),
                            "tanh" => n.tanh(),
                            _ => n,
                        };
                        Ok(Value::from_float(result))
                    }
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "min" | "max" => {
                if args.is_empty() {
                    return Err(Error::wrong_args(&format!("{}()", name), 1, args.len()));
                }
                let nums: std::result::Result<Vec<f64>, _> = args.iter()
                    .map(|v| v.as_float().ok_or_else(|| Error::type_mismatch("number", "non-numeric value")))
                    .collect();
                let nums = nums?;
                let result = if name == "min" {
                    nums.into_iter().fold(f64::INFINITY, f64::min)
                } else {
                    nums.into_iter().fold(f64::NEG_INFINITY, f64::max)
                };
                Ok(self.float_or_int(result))
            }
            "rand" => {
                // Simple pseudo-random: not cryptographic, but sufficient for Tcl compat
                // Use a linear congruential generator seeded from the address of interp
                let seed = (self.interp as *const Interp as usize) ^ self.pos;
                let val = ((seed.wrapping_mul(6364136223846793005).wrapping_add(1)) as f64)
                    / (usize::MAX as f64);
                Ok(Value::from_float(val.abs() % 1.0))
            }
            "srand" => {
                self.require_args(name, 1, args.len())?;
                // srand is mostly a no-op in our simple implementation
                Ok(Value::empty())
            }
            "isqrt" => {
                self.require_args(name, 1, args.len())?;
                match args[0].as_float() {
                    Some(n) if n >= 0.0 => Ok(Value::from_int((n.sqrt()) as i64)),
                    _ => Err(Error::runtime("domain error: argument not in valid range", crate::error::ErrorCode::InvalidOp)),
                }
            }
            _ => Err(Error::runtime(
                format!("unknown math function \"{}\"", name),
                crate::error::ErrorCode::InvalidOp,
            )),
        }
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
        if f.fract() == 0.0 && f.abs() < i64::MAX as f64 {
            Value::from_int(f as i64)
        } else {
            Value::from_float(f)
        }
    }

    fn require_args(&self, name: &str, expected: usize, actual: usize) -> Result<()> {
        if actual != expected {
            Err(Error::wrong_args(&format!("{}()", name), expected, actual))
        } else {
            Ok(())
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
mod tests {
    use super::*;

    #[test]
    fn test_arithmetic() {
        let mut interp = Interp::new();
        assert_eq!(eval_expr(&mut interp, "1 + 2").unwrap().as_int(), Some(3));
        assert_eq!(eval_expr(&mut interp, "10 - 3").unwrap().as_int(), Some(7));
        assert_eq!(eval_expr(&mut interp, "4 * 5").unwrap().as_int(), Some(20));
        assert_eq!(eval_expr(&mut interp, "15 / 3").unwrap().as_int(), Some(5));
    }

    #[test]
    fn test_comparison() {
        let mut interp = Interp::new();
        assert_eq!(eval_expr(&mut interp, "1 < 2").unwrap().as_bool(), Some(true));
        assert_eq!(eval_expr(&mut interp, "2 > 1").unwrap().as_bool(), Some(true));
        assert_eq!(eval_expr(&mut interp, "1 == 1").unwrap().as_bool(), Some(true));
        assert_eq!(eval_expr(&mut interp, "1 != 2").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_logical() {
        let mut interp = Interp::new();
        assert_eq!(eval_expr(&mut interp, "1 && 1").unwrap().as_bool(), Some(true));
        assert_eq!(eval_expr(&mut interp, "1 && 0").unwrap().as_bool(), Some(false));
        assert_eq!(eval_expr(&mut interp, "0 || 1").unwrap().as_bool(), Some(true));
        assert_eq!(eval_expr(&mut interp, "!0").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_functions() {
        let mut interp = Interp::new();
        assert_eq!(eval_expr(&mut interp, "abs(-5)").unwrap().as_int(), Some(5));
        assert_eq!(eval_expr(&mut interp, "sqrt(16)").unwrap().as_float(), Some(4.0));
        assert_eq!(eval_expr(&mut interp, "pow(2, 3)").unwrap().as_int(), Some(8));
    }

    #[test]
    fn test_variables() {
        let mut interp = Interp::new();
        interp.set_var("x", Value::from_int(10)).unwrap();
        assert_eq!(eval_expr(&mut interp, "$x + 5").unwrap().as_int(), Some(15));
    }
}
