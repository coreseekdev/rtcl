//! Standalone test for token functionality
//!
//! This is a simple test that can be run with `cargo run --example token_test`

fn main() {
    println!("Testing rtcl token system...\n");

    // Test 1: Newline normalization
    {
        let input = "a\nb\r\nc";
        let mut tz = rtcl_parser::rd::token::Tokenizer::new(input);
        assert_eq!(tz.next(), rtcl_parser::rd::token::Token::Other('a'));
        assert_eq!(tz.next(), rtcl_parser::rd::token::Token::Newline);
        assert_eq!(tz.next(), rtcl_parser::rd::token::Token::Other('b'));
        assert_eq!(tz.next(), rtcl_parser::rd::token::Token::Newline); // \r\n normalized
        assert_eq!(tz.next(), rtcl_parser::rd::token::Token::Other('c'));
        assert_eq!(tz.next(), rtcl_parser::rd::token::Token::Eof);
        println!("✓ Test 1: Newline normalization passed");
    }

    // Test 2: Standalone CR
    {
        let input = "a\rb";
        let mut tz = rtcl_parser::rd::token::Tokenizer::new(input);
        assert_eq!(tz.next(), rtcl_parser::rd::token::Token::Other('a'));
        assert_eq!(tz.next(), rtcl_parser::rd::token::Token::CarriageReturn);
        assert_eq!(tz.next(), rtcl_parser::rd::token::Token::Other('b'));
        println!("✓ Test 2: Standalone CR as whitespace passed");
    }

    // Test 3: Line tracking
    {
        let input = "a\nb\r\nc";
        let tz = rtcl_parser::rd::token::Tokenizer::new(input);
        assert_eq!(tz.line(), 1);
        println!("✓ Test 3: Line tracking passed");
    }

    // Test 4: Token properties
    {
        use rtcl_parser::rd::token::Token;
        assert!(Token::CarriageReturn.is_line_whitespace());
        assert!(Token::Whitespace.is_line_whitespace());
        assert!(!Token::Newline.is_line_whitespace());
        assert!(Token::Newline.is_any_whitespace());
        assert!(Token::Newline.is_command_end());
        assert!(Token::Semicolon.is_command_end());
        assert!(Token::Eof.is_command_end());
        println!("✓ Test 4: Token properties passed");
    }

    // Test 5: Cursor with newlines
    {
        use rtcl_parser::rd::Cursor;
        let mut cur = Cursor::new("a\nb\r\nc");
        assert!(matches!(cur.peek(), Token::Other('a')));
        cur.advance();
        assert_eq!(cur.peek(), Token::Newline);
        cur.advance();
        assert_eq!(cur.peek(), Token::Other('b'));
        cur.advance();
        assert_eq!(cur.peek(), Token::Newline); // \r\n normalized
        cur.advance();
        assert_eq!(cur.peek(), Token::Other('c'));
        println!("✓ Test 5: Cursor with normalized newlines passed");
    }

    println!("\n✅ All token tests passed!");
}
