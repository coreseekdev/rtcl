//! Type definitions and utilities

pub(crate) mod bignum;
pub mod expr;
pub(crate) mod expr_check;
pub mod expr_funcs;
pub mod expr_ops;

pub use expr::eval_expr;
