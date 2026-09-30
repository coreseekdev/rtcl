//! Type definitions and utilities

pub(crate) mod bignum;
pub mod expr;
pub(crate) mod expr_check;
pub(crate) mod expr_funcs;

pub use expr::eval_expr;
