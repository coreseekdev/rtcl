//! Tcl expression evaluator
//!
//! Supports arithmetic, comparison, and logical operations

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;

/// Evaluate a Tcl expression
pub fn eval_expr(interp: &mut Interp, expr: &str) -> Result<Value> {
    let mut parser = ExprParser::new(expr, interp);
    parser.parse_or()
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

    /// Parse or expression (lowest precedence)
    fn parse_or(&mut self) -> Result<Value> {
        let mut left = self.parse_and()?;

        while self.match_op("||") {
            let right = self.parse_and()?;
            left = Value::from_bool(left.is_true() || right.is_true());
        }

        Ok(left)
    }

    /// Parse and expression
    fn parse_and(&mut self) -> Result<Value> {
        let mut left = self.parse_comparison()?;

        while self.match_op("&&") {
            let right = self.parse_comparison()?;
            left = Value::from_bool(left.is_true() && right.is_true());
        }

        Ok(left)
    }

    /// Parse comparison expression
    fn parse_comparison(&mut self) -> Result<Value> {
        let mut left = self.parse_additive()?;

        loop {
            let op = if self.match_op("==") {
                "=="
            } else if self.match_op("!=") {
                "!="
            } else if self.match_op("<=") {
                "<="
            } else if self.match_op(">=") {
                ">="
            } else if self.match_op("<") {
                "<"
            } else if self.match_op(">") {
                ">"
            } else if self.match_op("eq") {
                "eq"
            } else if self.match_op("ne") {
                "ne"
            } else {
                break;
            };

            let right = self.parse_additive()?;

            left = match op {
                "==" => {
                    // Try numeric comparison first
                    match (left.as_float(), right.as_float()) {
                        (Some(a), Some(b)) => Value::from_bool((a - b).abs() < f64::EPSILON),
                        _ => Value::from_bool(left.as_str() == right.as_str()),
                    }
                }
                "!=" => {
                    match (left.as_float(), right.as_float()) {
                        (Some(a), Some(b)) => Value::from_bool((a - b).abs() >= f64::EPSILON),
                        _ => Value::from_bool(left.as_str() != right.as_str()),
                    }
                }
                "<=" => {
                    match (left.as_float(), right.as_float()) {
                        (Some(a), Some(b)) => Value::from_bool(a <= b),
                        _ => Value::from_bool(left.as_str() <= right.as_str()),
                    }
                }
                ">=" => {
                    match (left.as_float(), right.as_float()) {
                        (Some(a), Some(b)) => Value::from_bool(a >= b),
                        _ => Value::from_bool(left.as_str() >= right.as_str()),
                    }
                }
                "<" => {
                    match (left.as_float(), right.as_float()) {
                        (Some(a), Some(b)) => Value::from_bool(a < b),
                        _ => Value::from_bool(left.as_str() < right.as_str()),
                    }
                }
                ">" => {
                    match (left.as_float(), right.as_float()) {
                        (Some(a), Some(b)) => Value::from_bool(a > b),
                        _ => Value::from_bool(left.as_str() > right.as_str()),
                    }
                }
                "eq" => Value::from_bool(left.as_str() == right.as_str()),
                "ne" => Value::from_bool(left.as_str() != right.as_str()),
                _ => break,
            };
        }

        Ok(left)
    }

    /// Parse additive expression (+, -)
    fn parse_additive(&mut self) -> Result<Value> {
        let mut left = self.parse_multiplicative()?;

        loop {
            let op = if self.match_op("+") {
                '+'
            } else if self.match_op("-") {
                '-'
            } else {
                break;
            };

            let right = self.parse_multiplicative()?;

            left = match (left.as_float(), right.as_float()) {
                (Some(a), Some(b)) => {
                    let result = if op == '+' { a + b } else { a - b };
                    if result.fract() == 0.0 && result.abs() < i64::MAX as f64 {
                        Value::from_int(result as i64)
                    } else {
                        Value::from_float(result)
                    }
                }
                _ => {
                    return Err(Error::type_mismatch("number", "non-numeric value"));
                }
            };
        }

        Ok(left)
    }

    /// Parse multiplicative expression (*, /, %)
    fn parse_multiplicative(&mut self) -> Result<Value> {
        let mut left = self.parse_unary()?;

        loop {
            let op = if self.match_op("*") {
                '*'
            } else if self.match_op("/") {
                '/'
            } else if self.match_op("%") {
                '%'
            } else {
                break;
            };

            let right = self.parse_unary()?;

            left = match (left.as_float(), right.as_float()) {
                (Some(a), Some(b)) => {
                    if b == 0.0 {
                        return Err(Error::DivisionByZero);
                    }
                    let result = match op {
                        '*' => a * b,
                        '/' => a / b,
                        '%' => (a as i64 % b as i64) as f64,
                        _ => break,
                    };
                    if result.fract() == 0.0 && result.abs() < i64::MAX as f64 {
                        Value::from_int(result as i64)
                    } else {
                        Value::from_float(result)
                    }
                }
                _ => {
                    return Err(Error::type_mismatch("number", "non-numeric value"));
                }
            };
        }

        Ok(left)
    }

    /// Parse unary expression (!, -, +)
    fn parse_unary(&mut self) -> Result<Value> {
        if self.match_op("!") {
            let val = self.parse_unary()?;
            return Ok(Value::from_bool(!val.is_true()));
        }

        if self.match_op("-") {
            let val = self.parse_unary()?;
            match val.as_float() {
                Some(n) => {
                    if n.fract() == 0.0 && n.abs() < i64::MAX as f64 {
                        return Ok(Value::from_int(-(n as i64)));
                    }
                    return Ok(Value::from_float(-n));
                }
                None => return Err(Error::type_mismatch("number", "non-numeric value")),
            }
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
            let val = self.parse_or()?;
            self.expect(")")?;
            return Ok(val);
        }

        // Command substitution
        if self.peek() == '[' {
            self.advance(); // consume [
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
            // Execute the command
            return self.interp.eval(&cmd);
        }

        // Variable reference
        if self.peek() == '$' {
            self.advance();
            let name = self.parse_identifier();
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

    /// Parse a number
    fn parse_number(&mut self) -> Result<Value> {
        let mut s = String::new();

        // Handle hex
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

        // Decimal number
        while self.is_digit() {
            s.push(self.advance());
        }

        if self.peek() == '.' {
            s.push(self.advance());
            while self.is_digit() {
                s.push(self.advance());
            }
        }

        // Exponent
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

    /// Parse a string literal
    fn parse_string(&mut self) -> Result<Value> {
        let quote = self.advance();
        let mut s = String::new();

        while !self.is_at_end() && self.peek() != quote {
            if quote == '"' && self.peek() == '\\' {
                self.advance();
                if !self.is_at_end() {
                    s.push(self.parse_escape_char());
                }
            } else {
                s.push(self.advance());
            }
        }

        if !self.is_at_end() {
            self.advance(); // Closing quote
        }

        Ok(Value::from_str(&s))
    }

    /// Parse an escape character
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

    /// Parse an identifier
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

    /// Parse a function call
    fn parse_function_call(&mut self, name: &str) -> Result<Value> {
        self.expect("(")?;

        let mut args = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek() == ')' {
                break;
            }

            let arg = self.parse_or()?;
            args.push(arg);

            self.skip_whitespace();
            if self.peek() == ',' {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(")")?;

        // Built-in functions
        match name {
            "abs" => {
                if args.len() != 1 {
                    return Err(Error::wrong_args("abs()", 1, args.len()));
                }
                match args[0].as_float() {
                    Some(n) => {
                        let result = n.abs();
                        if result.fract() == 0.0 && result.abs() < i64::MAX as f64 {
                            Ok(Value::from_int(result as i64))
                        } else {
                            Ok(Value::from_float(result))
                        }
                    }
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "int" => {
                if args.len() != 1 {
                    return Err(Error::wrong_args("int()", 1, args.len()));
                }
                match args[0].as_float() {
                    Some(n) => Ok(Value::from_int(n as i64)),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "double" => {
                if args.len() != 1 {
                    return Err(Error::wrong_args("double()", 1, args.len()));
                }
                match args[0].as_float() {
                    Some(n) => Ok(Value::from_float(n)),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "round" => {
                if args.len() != 1 {
                    return Err(Error::wrong_args("round()", 1, args.len()));
                }
                match args[0].as_float() {
                    Some(n) => Ok(Value::from_int(n.round() as i64)),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "floor" => {
                if args.len() != 1 {
                    return Err(Error::wrong_args("floor()", 1, args.len()));
                }
                match args[0].as_float() {
                    Some(n) => Ok(Value::from_int(n.floor() as i64)),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "ceil" => {
                if args.len() != 1 {
                    return Err(Error::wrong_args("ceil()", 1, args.len()));
                }
                match args[0].as_float() {
                    Some(n) => Ok(Value::from_int(n.ceil() as i64)),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "sqrt" => {
                if args.len() != 1 {
                    return Err(Error::wrong_args("sqrt()", 1, args.len()));
                }
                match args[0].as_float() {
                    Some(n) => Ok(Value::from_float(n.sqrt())),
                    None => Err(Error::type_mismatch("number", "non-numeric value")),
                }
            }
            "pow" => {
                if args.len() != 2 {
                    return Err(Error::wrong_args("pow()", 2, args.len()));
                }
                match (args[0].as_float(), args[1].as_float()) {
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
            }
            "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "log" | "log10" | "exp" => {
                if args.len() != 1 {
                    return Err(Error::wrong_args(&format!("{}()", name), 1, args.len()));
                }
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
                let nums: Vec<f64> = args.iter()
                    .filter_map(|v| v.as_float())
                    .collect();
                if nums.len() != args.len() {
                    return Err(Error::type_mismatch("number", "non-numeric value"));
                }
                let result = if name == "min" {
                    nums.into_iter().fold(f64::INFINITY, f64::min)
                } else {
                    nums.into_iter().fold(f64::NEG_INFINITY, f64::max)
                };
                Ok(Value::from_float(result))
            }
            _ => Err(Error::runtime(
                format!("unknown function: {}", name),
                crate::error::ErrorCode::InvalidOp,
            )),
        }
    }

    // Helper methods

    fn peek(&self) -> char {
        self.chars.get(self.pos).copied().unwrap_or('\0')
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
