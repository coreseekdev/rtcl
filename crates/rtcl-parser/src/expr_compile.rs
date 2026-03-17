//! Expression inline compiler — compiles simple Tcl expressions to VM opcodes.
//!
//! Handles common patterns like `$x > 0`, `$i < 10`, `!$done`, `$x + 1`
//! at compile time, emitting native comparison/arithmetic opcodes instead
//! of `PushConst + EvalExpr`.
//!
//! Falls back to `None` for expressions that are too complex (function calls,
//! command substitution `[…]`, ternary `?:`, string literals with spaces, etc.).

use crate::bytecode::ByteCode;
use crate::opcode::OpCode;

/// Attempt to compile an expression string to inline opcodes.
///
/// Returns `true` if the expression was successfully compiled inline.
/// Returns `false` if the expression is too complex — caller should
/// fall back to `PushConst + EvalExpr`.
pub fn try_compile_expr(bytecode: &mut ByteCode, expr: &str, line: u32) -> bool {
    let tokens = match tokenize(expr) {
        Some(t) => t,
        None => return false,
    };
    if tokens.is_empty() {
        return false;
    }
    let mut parser = ExprCodegen { bytecode, tokens: &tokens, pos: 0, line };
    if parser.parse_or().is_err() {
        return false;
    }
    // Must have consumed all tokens
    parser.pos == parser.tokens.len()
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Int(i64),
    Float(f64),
    Var(String),
    // Operators
    Plus, Minus, Star, Slash, Percent, StarStar,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or, Not,
    BitAnd, BitOr, BitXor, BitNot, Shl, Shr,
    StrEq, StrNe,
    LParen, RParen,
}

fn tokenize(expr: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = expr.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    let len = chars.len();

    while i < len {
        // Skip whitespace
        if chars[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Variable reference: $name or ${name}
        if chars[i] == '$' {
            i += 1;
            if i >= len {
                return None;
            }
            if chars[i] == '{' {
                // ${name}
                i += 1;
                let start = i;
                while i < len && chars[i] != '}' {
                    i += 1;
                }
                if i >= len {
                    return None;
                }
                let name: String = chars[start..i].iter().collect();
                i += 1; // skip '}'
                // Reject array refs and nested expressions
                if name.contains('(') || name.contains('[') {
                    return None;
                }
                tokens.push(Token::Var(name));
            } else if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
                let start = i;
                while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                // Reject array element access $name(index)
                if i < len && chars[i] == '(' {
                    return None;
                }
                let name: String = chars[start..i].iter().collect();
                tokens.push(Token::Var(name));
            } else {
                return None; // unsupported $ usage
            }
            continue;
        }

        // Number (integer or float)
        if chars[i].is_ascii_digit() || (chars[i] == '-' && i + 1 < len && chars[i + 1].is_ascii_digit() && (tokens.is_empty() || matches!(tokens.last(), Some(Token::LParen | Token::Plus | Token::Minus | Token::Star | Token::Slash | Token::Percent | Token::Eq | Token::Ne | Token::Lt | Token::Gt | Token::Le | Token::Ge | Token::And | Token::Or | Token::Not | Token::BitAnd | Token::BitOr | Token::BitXor | Token::BitNot | Token::Shl | Token::Shr | Token::StrEq | Token::StrNe)))) {
            let start = i;
            if chars[i] == '-' {
                i += 1;
            }
            // Handle 0x hex, 0o octal, 0b binary
            if i < len && chars[i] == '0' && i + 1 < len {
                match chars[i + 1] {
                    'x' | 'X' => {
                        i += 2;
                        while i < len && chars[i].is_ascii_hexdigit() {
                            i += 1;
                        }
                        let s: String = chars[start..i].iter().collect();
                        let val = i64::from_str_radix(s.trim_start_matches('-').trim_start_matches("0x").trim_start_matches("0X"), 16).ok()?;
                        let val = if chars[start] == '-' { -val } else { val };
                        tokens.push(Token::Int(val));
                        continue;
                    }
                    'o' | 'O' => {
                        i += 2;
                        while i < len && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                        let s: String = chars[start..i].iter().collect();
                        let val = i64::from_str_radix(s.trim_start_matches('-').trim_start_matches("0o").trim_start_matches("0O"), 8).ok()?;
                        let val = if chars[start] == '-' { -val } else { val };
                        tokens.push(Token::Int(val));
                        continue;
                    }
                    'b' | 'B' => {
                        i += 2;
                        while i < len && (chars[i] == '0' || chars[i] == '1') {
                            i += 1;
                        }
                        let s: String = chars[start..i].iter().collect();
                        let val = i64::from_str_radix(s.trim_start_matches('-').trim_start_matches("0b").trim_start_matches("0B"), 2).ok()?;
                        let val = if chars[start] == '-' { -val } else { val };
                        tokens.push(Token::Int(val));
                        continue;
                    }
                    _ => {}
                }
            }
            while i < len && chars[i].is_ascii_digit() {
                i += 1;
            }
            // Check for float
            if i < len && chars[i] == '.' {
                i += 1;
                while i < len && chars[i].is_ascii_digit() {
                    i += 1;
                }
                // Scientific notation
                if i < len && (chars[i] == 'e' || chars[i] == 'E') {
                    i += 1;
                    if i < len && (chars[i] == '+' || chars[i] == '-') {
                        i += 1;
                    }
                    while i < len && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let s: String = chars[start..i].iter().collect();
                let val: f64 = s.parse().ok()?;
                tokens.push(Token::Float(val));
            } else {
                let s: String = chars[start..i].iter().collect();
                let val: i64 = s.parse().ok()?;
                tokens.push(Token::Int(val));
            }
            continue;
        }

        // Multi-char operators (check longest first)
        if i + 1 < len {
            let two: String = chars[i..i + 2].iter().collect();
            match two.as_str() {
                "**" => { tokens.push(Token::StarStar); i += 2; continue; }
                "==" => { tokens.push(Token::Eq); i += 2; continue; }
                "!=" => { tokens.push(Token::Ne); i += 2; continue; }
                "<=" => { tokens.push(Token::Le); i += 2; continue; }
                ">=" => { tokens.push(Token::Ge); i += 2; continue; }
                "&&" => { tokens.push(Token::And); i += 2; continue; }
                "||" => { tokens.push(Token::Or); i += 2; continue; }
                "<<" => { tokens.push(Token::Shl); i += 2; continue; }
                ">>" => { tokens.push(Token::Shr); i += 2; continue; }
                _ => {}
            }
        }

        // Word operators: eq, ne, in, ni — reject 'in'/'ni' as too complex
        if chars[i].is_ascii_alphabetic() {
            let start = i;
            while i < len && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            match word.as_str() {
                "eq" => { tokens.push(Token::StrEq); continue; }
                "ne" => { tokens.push(Token::StrNe); continue; }
                // "true"/"false" as boolean literals
                "true" => { tokens.push(Token::Int(1)); continue; }
                "false" => { tokens.push(Token::Int(0)); continue; }
                // Function calls, "in"/"ni", etc — bail out
                _ => return None,
            }
        }

        // Single-char operators
        match chars[i] {
            '+' => tokens.push(Token::Plus),
            '-' => tokens.push(Token::Minus),
            '*' => tokens.push(Token::Star),
            '/' => tokens.push(Token::Slash),
            '%' => tokens.push(Token::Percent),
            '<' => tokens.push(Token::Lt),
            '>' => tokens.push(Token::Gt),
            '!' => tokens.push(Token::Not),
            '&' => tokens.push(Token::BitAnd),
            '|' => tokens.push(Token::BitOr),
            '^' => tokens.push(Token::BitXor),
            '~' => tokens.push(Token::BitNot),
            '(' => tokens.push(Token::LParen),
            ')' => tokens.push(Token::RParen),
            // Anything else (strings, brackets, quotes) — bail out
            _ => return None,
        }
        i += 1;
    }

    Some(tokens)
}

// ---------------------------------------------------------------------------
// Code generator (recursive descent, same precedence as Tcl expr)
// ---------------------------------------------------------------------------

struct ExprCodegen<'a> {
    bytecode: &'a mut ByteCode,
    tokens: &'a [Token],
    pos: usize,
    line: u32,
}

type CResult = Result<(), ()>;

impl<'a> ExprCodegen<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&Token> {
        let t = self.tokens.get(self.pos);
        if t.is_some() { self.pos += 1; }
        t
    }

    fn match_tok(&mut self, tok: &Token) -> bool {
        if self.peek() == Some(tok) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    // -- Precedence levels (lowest to highest) --

    /// Logical OR `||`
    fn parse_or(&mut self) -> CResult {
        self.parse_and()?;
        while self.match_tok(&Token::Or) {
            self.parse_and()?;
            self.bytecode.emit(OpCode::Or, self.line);
        }
        Ok(())
    }

    /// Logical AND `&&`
    fn parse_and(&mut self) -> CResult {
        self.parse_bitor()?;
        while self.match_tok(&Token::And) {
            self.parse_bitor()?;
            self.bytecode.emit(OpCode::And, self.line);
        }
        Ok(())
    }

    /// Bitwise OR `|`
    fn parse_bitor(&mut self) -> CResult {
        self.parse_bitxor()?;
        while self.match_tok(&Token::BitOr) {
            self.parse_bitxor()?;
            self.bytecode.emit(OpCode::BitOr, self.line);
        }
        Ok(())
    }

    /// Bitwise XOR `^`
    fn parse_bitxor(&mut self) -> CResult {
        self.parse_bitand()?;
        while self.match_tok(&Token::BitXor) {
            self.parse_bitand()?;
            self.bytecode.emit(OpCode::BitXor, self.line);
        }
        Ok(())
    }

    /// Bitwise AND `&`
    fn parse_bitand(&mut self) -> CResult {
        self.parse_equality()?;
        while self.match_tok(&Token::BitAnd) {
            self.parse_equality()?;
            self.bytecode.emit(OpCode::BitAnd, self.line);
        }
        Ok(())
    }

    /// Equality: `==`, `!=`, `eq`, `ne`
    fn parse_equality(&mut self) -> CResult {
        self.parse_relational()?;
        loop {
            if self.match_tok(&Token::Eq) {
                self.parse_relational()?;
                self.bytecode.emit(OpCode::Eq, self.line);
            } else if self.match_tok(&Token::Ne) {
                self.parse_relational()?;
                self.bytecode.emit(OpCode::Ne, self.line);
            } else if self.match_tok(&Token::StrEq) {
                self.parse_relational()?;
                self.bytecode.emit(OpCode::StrEq, self.line);
            } else if self.match_tok(&Token::StrNe) {
                self.parse_relational()?;
                self.bytecode.emit(OpCode::StrNe, self.line);
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Relational: `<`, `>`, `<=`, `>=`
    fn parse_relational(&mut self) -> CResult {
        self.parse_shift()?;
        loop {
            if self.match_tok(&Token::Lt) {
                self.parse_shift()?;
                self.bytecode.emit(OpCode::Lt, self.line);
            } else if self.match_tok(&Token::Gt) {
                self.parse_shift()?;
                self.bytecode.emit(OpCode::Gt, self.line);
            } else if self.match_tok(&Token::Le) {
                self.parse_shift()?;
                self.bytecode.emit(OpCode::Le, self.line);
            } else if self.match_tok(&Token::Ge) {
                self.parse_shift()?;
                self.bytecode.emit(OpCode::Ge, self.line);
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Shift: `<<`, `>>`
    fn parse_shift(&mut self) -> CResult {
        self.parse_add()?;
        loop {
            if self.match_tok(&Token::Shl) {
                self.parse_add()?;
                self.bytecode.emit(OpCode::Shl, self.line);
            } else if self.match_tok(&Token::Shr) {
                self.parse_add()?;
                self.bytecode.emit(OpCode::Shr, self.line);
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Additive: `+`, `-`
    fn parse_add(&mut self) -> CResult {
        self.parse_mul()?;
        loop {
            if self.match_tok(&Token::Plus) {
                self.parse_mul()?;
                self.bytecode.emit(OpCode::Add, self.line);
            } else if self.match_tok(&Token::Minus) {
                self.parse_mul()?;
                self.bytecode.emit(OpCode::Sub, self.line);
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Multiplicative: `*`, `/`, `%`
    fn parse_mul(&mut self) -> CResult {
        self.parse_power()?;
        loop {
            if self.match_tok(&Token::Star) {
                self.parse_power()?;
                self.bytecode.emit(OpCode::Mul, self.line);
            } else if self.match_tok(&Token::Slash) {
                self.parse_power()?;
                self.bytecode.emit(OpCode::Div, self.line);
            } else if self.match_tok(&Token::Percent) {
                self.parse_power()?;
                self.bytecode.emit(OpCode::Mod, self.line);
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Power: `**` (right-associative)
    fn parse_power(&mut self) -> CResult {
        self.parse_unary()?;
        if self.match_tok(&Token::StarStar) {
            self.parse_power()?; // right-associative
            self.bytecode.emit(OpCode::Pow, self.line);
        }
        Ok(())
    }

    /// Unary: `-`, `+`, `!`, `~`
    fn parse_unary(&mut self) -> CResult {
        if self.match_tok(&Token::Minus) {
            self.parse_unary()?;
            self.bytecode.emit(OpCode::Neg, self.line);
            Ok(())
        } else if self.match_tok(&Token::Plus) {
            self.parse_unary()
        } else if self.match_tok(&Token::Not) {
            self.parse_unary()?;
            self.bytecode.emit(OpCode::Not, self.line);
            Ok(())
        } else if self.match_tok(&Token::BitNot) {
            self.parse_unary()?;
            self.bytecode.emit(OpCode::BitNot, self.line);
            Ok(())
        } else {
            self.parse_primary()
        }
    }

    /// Primary: integer, float, variable, `(expr)`
    fn parse_primary(&mut self) -> CResult {
        match self.advance() {
            Some(Token::Int(n)) => {
                let n = *n;
                self.bytecode.emit(OpCode::PushInt(n), self.line);
                Ok(())
            }
            Some(Token::Float(_)) => {
                // Float not supported in VM integer arithmetic — bail
                Err(())
            }
            Some(Token::Var(name)) => {
                let name = name.clone();
                let idx = self.bytecode.add_const(&name);
                self.bytecode.emit(OpCode::LoadVar(idx), self.line);
                Ok(())
            }
            Some(Token::LParen) => {
                self.parse_or()?;
                if !self.match_tok(&Token::RParen) {
                    return Err(());
                }
                Ok(())
            }
            _ => Err(()),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_expr(expr: &str) -> Option<Vec<OpCode>> {
        let mut bc = ByteCode::new();
        if try_compile_expr(&mut bc, expr, 1) {
            Some(bc.ops().to_vec())
        } else {
            None
        }
    }

    #[test]
    fn simple_comparison() {
        let ops = compile_expr("$x > 0").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::LoadVar(_))));
        assert!(ops.iter().any(|o| matches!(o, OpCode::PushInt(0))));
        assert!(ops.iter().any(|o| matches!(o, OpCode::Gt)));
    }

    #[test]
    fn simple_less_than() {
        let ops = compile_expr("$i < 10").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::LoadVar(_))));
        assert!(ops.iter().any(|o| matches!(o, OpCode::PushInt(10))));
        assert!(ops.iter().any(|o| matches!(o, OpCode::Lt)));
    }

    #[test]
    fn equality() {
        let ops = compile_expr("$x == 1").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::Eq)));
    }

    #[test]
    fn logical_and() {
        let ops = compile_expr("$x > 0 && $y < 10").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::Gt)));
        assert!(ops.iter().any(|o| matches!(o, OpCode::Lt)));
        assert!(ops.iter().any(|o| matches!(o, OpCode::And)));
    }

    #[test]
    fn unary_not() {
        let ops = compile_expr("!$done").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::LoadVar(_))));
        assert!(ops.iter().any(|o| matches!(o, OpCode::Not)));
    }

    #[test]
    fn arithmetic() {
        let ops = compile_expr("$x + 1").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::Add)));
    }

    #[test]
    fn string_eq() {
        let ops = compile_expr("$x eq $y").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::StrEq)));
    }

    #[test]
    fn constant_expr() {
        let ops = compile_expr("2 > 3").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::PushInt(2))));
        assert!(ops.iter().any(|o| matches!(o, OpCode::PushInt(3))));
        assert!(ops.iter().any(|o| matches!(o, OpCode::Gt)));
    }

    #[test]
    fn single_constant() {
        let ops = compile_expr("1").unwrap();
        assert_eq!(ops, vec![OpCode::PushInt(1)]);
    }

    #[test]
    fn parenthesized() {
        let ops = compile_expr("($x + 1) * 2").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::Add)));
        assert!(ops.iter().any(|o| matches!(o, OpCode::Mul)));
    }

    #[test]
    fn rejects_command_sub() {
        assert!(compile_expr("[expr 1+1]").is_none());
    }

    #[test]
    fn rejects_string_literal() {
        assert!(compile_expr("\"hello world\"").is_none());
    }

    #[test]
    fn rejects_function_call() {
        assert!(compile_expr("abs($x)").is_none());
    }

    #[test]
    fn rejects_ternary() {
        assert!(compile_expr("$x ? 1 : 0").is_none());
    }

    #[test]
    fn hex_literal() {
        let ops = compile_expr("$x == 0xFF").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::PushInt(255))));
    }

    #[test]
    fn braced_var() {
        let ops = compile_expr("${count} > 0").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::LoadVar(_))));
        assert!(ops.iter().any(|o| matches!(o, OpCode::Gt)));
    }

    #[test]
    fn boolean_keywords() {
        let ops = compile_expr("true").unwrap();
        assert_eq!(ops, vec![OpCode::PushInt(1)]);
        let ops = compile_expr("false").unwrap();
        assert_eq!(ops, vec![OpCode::PushInt(0)]);
    }

    #[test]
    fn power_operator() {
        let ops = compile_expr("$x ** 2").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::Pow)));
    }

    #[test]
    fn bitwise_ops() {
        let ops = compile_expr("$x & 0xFF").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::BitAnd)));
    }

    #[test]
    fn complex_expr() {
        let ops = compile_expr("$i >= 0 && $i < $n").unwrap();
        assert!(ops.iter().any(|o| matches!(o, OpCode::Ge)));
        assert!(ops.iter().any(|o| matches!(o, OpCode::Lt)));
        assert!(ops.iter().any(|o| matches!(o, OpCode::And)));
    }
}
