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
    // tclsh's parser rejects malformed expressions before any
    // evaluation; run the same syntax pre-pass and log its errorInfo
    // frame the way the C parser does (ERR_ALREADY_LOGGED: the harness
    // continues the accumulated info with `invoked from within`).
    if let Err(msg) = super::expr_check::check_expr(expr) {
        interp.err_info =
            Some(format!("{}\n{}", msg, super::expr_check::parsing_frame(expr)));
        interp.err_fresh = false;
        return Err(Error::Msg(msg));
    }
    let mut parser = ExprParser::new(expr, interp);
    let result = parser.parse_ternary()?;
    canonicalize_result(result)
}

/// Tcl: when the result of an expression is a string that looks like a
/// number, it is converted to the number's canonical form — `expr {$x}`
/// with x="1e15" yields `1000000000000000.0`, `" 1"` yields `1`, `0x10`
/// yields `16`. Non-numeric strings pass through unchanged. (Numeric
/// *operators* convert their operands individually; `eq`/`ne` keep
/// comparing the original strings.)
fn canonicalize_result(v: Value) -> Result<Value> {
    if v.type_name() != "string" {
        return Ok(v);
    }
    if let Some(i) = v.as_int() {
        return Ok(Value::from_int(i));
    }
    // A plain decimal integer beyond i64 is a Tcl bignum; keep its exact
    // text rather than degrading it to a rounded double (E4).
    let t = v.as_str().trim();
    let digits = t.strip_prefix(['-', '+']).unwrap_or(t);
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        return Ok(v);
    }
    if let Some(f) = v.as_float() {
        // A string that parses to NaN cannot canonicalize — tclsh raises
        // a domain error (`expr {"nan"}` / bareword nan, probed bn7).
        if f.is_nan() {
            return Err(super::expr_funcs::domain_error());
        }
        return Ok(super::expr_funcs::float_value(f));
    }
    Ok(v)
}

/// Expression parser
struct ExprParser<'a> {
    chars: Vec<char>,
    pos: usize,
    interp: &'a mut Interp,
    /// Depth of math-function argument lists: a bare `nan` is accepted as
    /// a string literal inside them (`expr int(NaN)` errors inside int(),
    /// while a top-level `expr nan` is a domain error).
    func_arg_depth: u32,
}

impl<'a> ExprParser<'a> {
    fn new(expr: &str, interp: &'a mut Interp) -> Self {
        ExprParser {
            chars: expr.chars().collect(),
            pos: 0,
            interp,
            func_arg_depth: 0,
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
                left = self.int_bitop(&left, &right, '|')?;
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
                left = self.int_bitop(&left, &right, '^')?;
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
                left = self.int_bitop(&left, &right, '&')?;
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

    /// Shift: `<<`, `>>` (Tcl) plus rtcl extensions `<<<`, `>>>` (rotates).
    ///
    /// Tcl semantics (tclsh 8.6.17): a negative shift count errors with
    /// "negative shift argument"; `<<` widens to bignum when the result
    /// overflows i64 (`1 << 63` is the positive bignum 2^63); `>>` is an
    /// arithmetic shift that saturates past the width (`-1 >> 64` is -1);
    /// shift counts so large the result exceeds the bignum range error
    /// with "integer value too large to represent".
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
            } else if self.match_op(">>>") {
                let right = self.parse_additive()?;
                let a = self.as_int_val(&left)? as u64;
                let b = self.as_int_val(&right)? as u32;
                let bits = 64u32;
                let shift = b % bits;
                let rotated = if shift == 0 { a } else { (a >> shift) | (a << (bits - shift)) };
                left = Value::from_int(rotated as i64);
            } else if self.match_op("<<") || self.match_op(">>") {
                let shl = self.chars[self.pos - 1] == '<';
                let right = self.parse_additive()?;
                left = self.int_shift(&left, &right, shl)?;
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// One `<<`/`>>` operation with Tcl's bignum-widening semantics.
    fn int_shift(&self, left: &Value, right: &Value, shl: bool) -> Result<Value> {
        use num_traits::Signed;
        use super::bignum::{int_rep, to_value, IntRep};
        let op = if shl { "<<" } else { ">>" };
        // tclsh shift operands must be integers: a float operand errors
        // with the operator named, separately from other non-numeric
        // strings (`1.5 << 2` vs `"abc" << 1`).
        let int_operand = |v: &Value| -> Result<i64> {
            if let Some(i) = v.as_int() {
                return Ok(i);
            }
            if v.as_float().is_some() {
                return Err(Error::Msg(format!(
                    "can't use floating-point value as operand of \"{op}\""
                )));
            }
            Err(Error::Msg(format!(
                "can't use non-numeric string as operand of \"{op}\""
            )))
        };
        if int_rep(left).is_none() {
            int_operand(left)?;
        }
        // Count: i64 fast path; a bignum count behaves like an unbounded
        // one (>> saturates below, << errors above — `-0x8000000000000001
        // >> 0x8000000000000000` is -1, expr-48.1).
        let count = match int_rep(right) {
            Some(IntRep::I64(c)) => c,
            Some(big) => {
                if big.to_big().is_negative() {
                    return Err(Error::Msg("negative shift argument".to_string()));
                }
                i64::MAX
            }
            None => int_operand(right)?,
        };
        if count < 0 {
            return Err(Error::Msg("negative shift argument".to_string()));
        }
        // Bignum cap: results wider than ~2^27 bits exceed tclsh's
        // bignum range ("integer value too large to represent", probed
        // with `1 << 0x8000000000000000`).
        if shl && count > 1 << 27 {
            return Err(Error::Msg(
                "integer value too large to represent".to_string(),
            ));
        }
        match int_rep(left) {
            Some(IntRep::I64(a)) if !shl && count >= 64 => {
                Ok(Value::from_int(if a < 0 { -1 } else { 0 }))
            }
            Some(IntRep::Big(b)) => {
                let shifted = if shl {
                    b << (count as usize)
                } else if count as u64 >= b.bits() as u64 {
                    // Saturate without building a 2^count denominator.
                    if b.is_negative() {
                        num_bigint::BigInt::from(-1)
                    } else {
                        num_bigint::BigInt::from(0)
                    }
                } else {
                    b >> (count as usize)
                };
                Ok(to_value(IntRep::Big(shifted)))
            }
            Some(IntRep::I64(a)) if shl => {
                let wide = ((a as u64) << (count as u32)) as i64;
                if count < 64 && (wide >> count) == a {
                    Ok(Value::from_int(wide))
                } else {
                    // Overflow: widen to an exact bignum (1 << 63 is
                    // 9223372036854775808, not i64::MIN).
                    Ok(to_value(IntRep::Big(
                        num_bigint::BigInt::from(a) << (count as usize),
                    )))
                }
            }
            Some(IntRep::I64(a)) => Ok(Value::from_int(a >> (count as u32))),
            None => Err(Error::Msg(format!(
                "can't use non-numeric string as operand of \"{op}\""
            ))),
        }
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
                left = self.numeric_binop(&left, &right, '/')?;
            } else if self.peek() == '%' {
                self.advance();
                let right = self.parse_power()?;
                left = self.int_mod(&left, &right)?;
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
                    // 0**0 is 1 (expr-23.15); 0**n is 0 (expr-23.14).
                    0 if b == 0 => Ok(Value::from_int(1)),
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
                        // Overflow widens to an exact bignum (expr-23.48:
                        // 2**81 is 2417851639229258349412352).
                        None => Ok(super::bignum::to_value(
                            super::bignum::IntRep::Big(
                                num_bigint::BigInt::from(a)
                                    .pow(b as u32),
                            ),
                        )),
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
            if let Some(rep) = super::bignum::int_rep(&val) {
                // ~x is -x-1 in two's complement (~2^63 is -9223372036854775809).
                return Ok(super::bignum::to_value(super::bignum::IntRep::Big(
                    -rep.to_big() - num_bigint::BigInt::from(1),
                )));
            }
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
                || self.peek() == '~'
                || self.peek() == '!'
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
                if let Some(rep) = super::bignum::int_rep(&val) {
                    return Ok(match rep {
                        super::bignum::IntRep::I64(i) => match i.checked_neg() {
                            Some(r) => Value::from_int(r),
                            None => super::bignum::to_value(super::bignum::IntRep::Big(
                                num_bigint::BigInt::from(2i8).pow(63u32),
                            )),
                        },
                        big => super::bignum::to_value(super::bignum::IntRep::Big(
                            -big.to_big(),
                        )),
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
        // Boolean literals keep their string form, including tclsh's
        // unique-prefix abbreviations (`expr bool(y)` lexes y as a bool
        // constant, expr-31.0.4.0; `expr {t}` renders "t").
        if super::expr_funcs::bool_from_string(&ident).is_some() {
            return Ok(Value::from_str(&ident));
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
                // A bareword nan is a string value in tclsh: `expr nan`
                // domain-errors at canonicalization, `nan + 0` reports the
                // operand error, `int(nan)` reports the NaN error.
                return Ok(Value::from_str(&ident));
            }
            return Ok(Value::from_float(n));
        }

        // Return as string
        Ok(Value::from_str(&ident))
    }

    // -- Number / string / identifier parsing --------------------------------

    fn parse_number(&mut self) -> Result<Value> {
        let mut s = String::new();

        // Handle hex / binary / octal (0x / 0b / 0o) and legacy octal.
        // In-range radix literals keep their original text (expr's `eq`
        // compares the raw string: `01eq1` is false, expr-8.17) while the
        // top-level result canonicalizes to decimal; oversized literals
        // widen to exact bignums rendered in decimal (expr-43.11).
        if self.peek() == '0' && self.pos + 1 < self.chars.len() {
            let next = self.chars[self.pos + 1];
            if next == 'x' || next == 'X' {
                s.push(self.advance());
                s.push(self.advance());
                while self.is_hex_digit() {
                    s.push(self.advance());
                }
                return self.radix_literal(&s, 16, &s[2..]);
            }
            if next == 'b' || next == 'B' {
                s.push(self.advance());
                s.push(self.advance());
                while self.peek() == '0' || self.peek() == '1' {
                    s.push(self.advance());
                }
                return self.radix_literal(&s, 2, &s[2..]);
            }
            if next == 'o' || next == 'O' {
                s.push(self.advance());
                s.push(self.advance());
                while self.peek() >= '0' && self.peek() <= '7' {
                    s.push(self.advance());
                }
                return self.radix_literal(&s, 8, &s[2..]);
            }
            if next.is_ascii_digit() {
                // Legacy octal: `017` is 15; a digit 8/9 makes the whole
                // token an invalid bareword with tclsh's octal hint —
                // unless a decimal continuation follows (`028.1` is 28.1,
                // `08e2` is 800.0, `085.` is 85.0), in which case the
                // whole token is a double.
                let octal_start = self.pos;
                s.push(self.advance());
                while self.peek() >= '0' && self.peek() <= '7' {
                    s.push(self.advance());
                }
                let hit89 = self.peek() == '8' || self.peek() == '9';
                // Lookahead without consuming: digits, optional `.`+digits,
                // optional exponent.  A decimal continuation makes the
                // whole token a double regardless of a valid octal prefix
                // (`077.5` is 77.5, `028.1` is 28.1, `08e2` is 800.0,
                // `085.` is 85.0).
                let mut j = self.pos;
                let mut saw_dot = false;
                let mut saw_exp = false;
                while j < self.chars.len() && self.chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j < self.chars.len() && self.chars[j] == '.' {
                    saw_dot = true;
                    j += 1;
                    while j < self.chars.len() && self.chars[j].is_ascii_digit() {
                        j += 1;
                    }
                }
                if j < self.chars.len() && (self.chars[j] == 'e' || self.chars[j] == 'E') {
                    let mut k = j + 1;
                    if k < self.chars.len() && (self.chars[k] == '+' || self.chars[k] == '-') {
                        k += 1;
                    }
                    if k < self.chars.len() && self.chars[k].is_ascii_digit() {
                        saw_exp = true;
                    }
                }
                if saw_dot || saw_exp {
                    // Fall through to the decimal scanner below, from the
                    // token start.
                    self.pos = octal_start;
                    s.clear();
                } else if hit89 {
                    // The whole digit run belongs to the offending token.
                    while self.is_digit() {
                        s.push(self.advance());
                    }
                    return Err(self.invalid_octal_error(&s));
                } else {
                    return self.radix_literal(&s, 8, &s[1..]);
                }
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
                // Tcl widens oversized integer literals to exact bignums.
                Err(_) => match num_bigint::BigInt::parse_bytes(s.as_bytes(), 10) {
                    // Bignum values carry the canonical decimal string.
                    Some(_) => Ok(Value::from_str(&s)),
                    None => Ok(super::expr_funcs::float_value(f64::INFINITY)),
                },
            }
        }
    }

    /// Resolve a radix literal (`0x…`/`0b…`/`0o…`/legacy octal) of the
    /// given text whose digits start at `body`.  In-range values keep the
    /// raw text (string rep is preserved for `eq`; the top-level result
    /// still canonicalizes); oversized values widen to an exact bignum in
    /// decimal form.
    fn radix_literal(&mut self, s: &str, radix: u32, body: &str) -> Result<Value> {
        if i64::from_str_radix(body, radix).is_ok() {
            return Ok(Value::from_str(s));
        }
        match num_bigint::BigInt::parse_bytes(body.as_bytes(), radix) {
            Some(big) => Ok(Value::from_str(&big.to_string())),
            None => Ok(super::expr_funcs::float_value(f64::INFINITY)),
        }
    }

    /// tclsh's invalid-octal bareword error, e.g. for `expr 018`:
    /// `invalid bareword "018" ... or ... (invalid octal number?)`.
    fn invalid_octal_error(&self, token: &str) -> Error {
        let expr: String = self.chars.iter().collect();
        Error::Msg(format!(
            "invalid bareword \"{token}\"\nin expression \"{expr}\";\nshould be \"${token}\" or \"{{{token}}}\" or \"{token}(...)\" or ... (invalid octal number?)"
        ))
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

    /// Tcl backslash substitution inside double-quoted expr strings
    /// (tclsh: `expr {"\374" eq "\xFC"}` — both are ü, expr-8.13).
    fn parse_escape_char(&mut self) -> char {
        match self.advance() {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'f' => '\u{0c}',
            'v' => '\u{0b}',
            'b' => '\u{08}',
            'a' => '\u{07}',
            '\\' => '\\',
            '"' => '"',
            'x' => {
                // \xHH — all hex digits consumed, last two used.
                let mut val: u32 = 0;
                let mut count = 0;
                while count < 8 && self.is_hex_digit() {
                    let d = self.advance().to_digit(16).unwrap_or(0);
                    val = val * 16 + d;
                    count += 1;
                }
                if count == 0 {
                    'x'
                } else {
                    char::from_u32(val & 0xff).unwrap_or('\u{fffd}')
                }
            }
            'u' => {
                // \uHHHH — up to four hex digits.
                let mut val: u32 = 0;
                let mut count = 0;
                while count < 4 && self.is_hex_digit() {
                    let d = self.advance().to_digit(16).unwrap_or(0);
                    val = val * 16 + d;
                    count += 1;
                }
                if count == 0 {
                    'u'
                } else {
                    char::from_u32(val).unwrap_or('\u{fffd}')
                }
            }
            c if ('0'..='7').contains(&c) => {
                // \NNN — up to three octal digits, the first consumed.
                let mut val = c.to_digit(8).unwrap_or(0);
                let mut count = 1;
                while count < 3 && ('0'..='7').contains(&self.peek()) {
                    val = val * 8 + self.advance().to_digit(8).unwrap_or(0);
                    count += 1;
                }
                char::from_u32(val & 0xff).unwrap_or('\u{fffd}')
            }
            '\n' => ' ',
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
        self.func_arg_depth += 1;
        let parsed = (|| -> Result<()> {
            loop {
                self.skip_whitespace();
                if self.peek() == ')' { break; }
                let arg = self.parse_ternary()?;
                args.push(arg);
                self.skip_whitespace();
                if self.peek() == ',' { self.advance(); } else { break; }
            }
            Ok(())
        })();
        self.func_arg_depth -= 1;
        parsed?;
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

    /// Bitwise op on exact integers; bignum operands use two's-complement
    /// BigInt ops (`-2 & 0xff` is 254).  Non-integer operands keep the
    /// legacy float-truncation path.
    fn int_bitop(&self, left: &Value, right: &Value, op: char) -> Result<Value> {
        if let (Some(ia), Some(ib)) = (
            super::bignum::int_rep(left),
            super::bignum::int_rep(right),
        ) {
            use num_bigint::BigInt;
            let (a, b) = (ia.to_big(), ib.to_big());
            return Ok(super::bignum::to_value(super::bignum::IntRep::Big(
                match op {
                    '|' => a | b,
                    '^' => a ^ b,
                    _ => a & b,
                },
            )));
        }
        // Non-int operands: tclsh rejects floats and other non-numeric
        // strings with the operator in the message.
        let int_operand = |v: &Value| -> Result<i64> {
            if let Some(i) = v.as_int() {
                return Ok(i);
            }
            if v.as_float().is_some() {
                return Err(Error::Msg(format!(
                    "can't use floating-point value as operand of \"{op}\""
                )));
            }
            Err(Error::Msg(format!(
                "can't use non-numeric string as operand of \"{op}\""
            )))
        };
        let a = int_operand(left)?;
        let b = int_operand(right)?;
        Ok(Value::from_int(match op {
            '|' => a | b,
            '^' => a ^ b,
            _ => a & b,
        }))
    }

    /// `%` with Tcl floor semantics; bignum operands stay exact.
    fn int_mod(&self, left: &Value, right: &Value) -> Result<Value> {
        use super::bignum::{floor_mod as big_mod, int_rep, to_value, IntRep};
        if let (Some(ia), Some(ib)) = (int_rep(left), int_rep(right)) {
            if ib.is_zero() {
                return Err(Error::DivisionByZero);
            }
            if let (IntRep::I64(a), IntRep::I64(b)) = (&ia, &ib) {
                // i64::MIN % -1 would overflow in Rust; the result is 0
                if !(*a == i64::MIN && *b == -1) {
                    return Ok(Value::from_int(floor_mod(*a, *b)));
                }
                return Ok(Value::from_int(0));
            }
            return Ok(to_value(IntRep::Big(big_mod(&ia.to_big(), &ib.to_big()))));
        }
        // Non-int operands: tclsh distinguishes a float operand
        // (`1.5 % 2` → can't use floating-point value as operand of "%")
        // from any other non-numeric one (`true % 2` → can't use
        // non-numeric string as operand of "%").
        let int_operand = |v: &Value| -> Result<i64> {
            if let Some(i) = v.as_int() {
                return Ok(i);
            }
            if v.as_float().is_some() {
                return Err(Error::Msg(
                    "can't use floating-point value as operand of \"%\"".to_string(),
                ));
            }
            Err(Error::Msg(
                "can't use non-numeric string as operand of \"%\"".to_string(),
            ))
        };
        let a = int_operand(left)?;
        let b = int_operand(right)?;
        if b == 0 { return Err(Error::DivisionByZero); }
        Ok(Value::from_int(if a == i64::MIN && b == -1 { 0 } else { floor_mod(a, b) }))
    }

    /// Convert `Value` to i64, returning an error if not numeric.
    fn as_int_val(&self, v: &Value) -> Result<i64> {
        v.as_int().or_else(|| v.as_float().map(|f| f as i64))
            .ok_or_else(|| Error::type_mismatch("integer", v.as_str()))
    }

    /// Numeric binary operation. Integer operands use integer arithmetic
    /// (Tcl semantics: `/` is floor division, result is int); on i64
    /// overflow the operation widens to an exact bignum (`9223372036854775807
    /// + 1` is 9223372036854775808, tclsh-style). If either side is float,
    /// compute in f64 and the result stays a float.
    fn numeric_binop(&self, left: &Value, right: &Value, op: char) -> Result<Value> {
        use num_bigint::BigInt;
        use num_integer::Integer;
        use super::bignum::{floor_div as big_div, int_rep, to_value, IntRep};
        // NaN operands can't take part in arithmetic (`"nan" + 0` →
        // can't use non-numeric floating-point value as operand of "+").
        if left.as_float().map_or(false, |f| f.is_nan())
            || right.as_float().map_or(false, |f| f.is_nan())
        {
            return Err(Error::Msg(format!(
                "can't use non-numeric floating-point value as operand of \"{op}\""
            )));
        }
        if let (Some(ia), Some(ib)) = (int_rep(left), int_rep(right)) {
            if let (IntRep::I64(a), IntRep::I64(b)) = (&ia, &ib) {
                let widen = |x: BigInt| Ok(to_value(IntRep::Big(x)));
                let (ba, bb) = (ia.to_big(), ib.to_big());
                return match op {
                    '+' => match a.checked_add(*b) {
                        Some(r) => Ok(Value::from_int(r)),
                        None => widen(ba + bb),
                    },
                    '-' => match a.checked_sub(*b) {
                        Some(r) => Ok(Value::from_int(r)),
                        None => widen(ba - bb),
                    },
                    '*' => match a.checked_mul(*b) {
                        Some(r) => Ok(Value::from_int(r)),
                        None => widen(ba * bb),
                    },
                    '/' => {
                        if *b == 0 {
                            return Err(Error::DivisionByZero);
                        }
                        // i64::MIN / -1 overflows in Rust; Tcl widens to bignum
                        if *a == i64::MIN && *b == -1 {
                            return widen(ba.div_floor(&bb));
                        }
                        Ok(Value::from_int(floor_div(*a, *b)))
                    }
                    _ => Err(Error::runtime("unknown op", crate::error::ErrorCode::InvalidOp)),
                };
            }
            // At least one bignum operand: exact BigInt arithmetic.
            let (ba, bb) = (ia.to_big(), ib.to_big());
            return match op {
                '+' => Ok(to_value(IntRep::Big(ba + bb))),
                '-' => Ok(to_value(IntRep::Big(ba - bb))),
                '*' => Ok(to_value(IntRep::Big(ba * bb))),
                '/' => {
                    if ib.is_zero() {
                        return Err(Error::DivisionByZero);
                    }
                    Ok(to_value(IntRep::Big(big_div(&ba, &bb))))
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
                        // tclsh follows IEEE here: x/0.0 is ±Inf (5/0.0 →
                        // Inf, 1.0/-0.0 → -Inf); only a zero dividend gives
                        // the NaN result tclsh refuses to build — 0/0.0 →
                        // "domain error: argument not in valid range".
                        if b == 0.0 && a == 0.0 {
                            return Err(super::expr_funcs::domain_error());
                        }
                        a / b
                    }
                    _ => return Err(Error::runtime("unknown op", crate::error::ErrorCode::InvalidOp)),
                };
                Ok(super::expr_funcs::float_value(result))
            }
            _ => Err(Error::Msg(format!(
                "can't use non-numeric string as operand of \"{op}\""
            ))),
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
