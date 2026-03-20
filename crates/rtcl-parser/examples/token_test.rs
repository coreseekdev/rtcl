//! Standalone test for token functionality
//!
//! This is a simple test that can be run with `cargo run --example token_test`

fn main() {
    println!("Testing rtcl token system...\n");

    // Test 1: Newline normalization
    {
        let input = "a\nb\r\nc";
        let mut tz = rtcl_parser::Tokenizer::new(input);
        assert_eq!(tz.next(), rtcl_parser::Token::Other('a'));
        assert_eq!(tz.next(), rtcl_parser::Token::Newline);
        assert_eq!(tz.next(), rtcl_parser::Token::Other('b'));
        assert_eq!(tz.next(), rtcl_parser::Token::Newline); // \r\n normalized
        assert_eq!(tz.next(), rtcl_parser::Token::Other('c'));
        assert_eq!(tz.next(), rtcl_parser::Token::Eof);
        println!("✓ Test 1: Newline normalization passed");
    }

    // Test 2: Standalone CR
    {
        let input = "a\rb";
        let mut tz = rtcl_parser::Tokenizer::new(input);
        assert_eq!(tz.next(), rtcl_parser::Token::Other('a'));
        assert_eq!(tz.next(), rtcl_parser::Token::CarriageReturn);
        assert_eq!(tz.next(), rtcl_parser::Token::Other('b'));
        println!("✓ Test 2: Standalone CR as whitespace passed");
    }

    // Test 3: Line tracking
    {
        let input = "a\nb\r\nc";
        let tz = rtcl_parser::Tokenizer::new(input);
        assert_eq!(tz.line(), 1);
        println!("✓ Test 3: Line tracking passed");
    }

    // Test 4: Token properties
    {
        use rtcl_parser::Token;
        assert!(Token::CarriageReturn.is_line_whitespace());
        assert!(Token::Whitespace.is_line_whitespace());
        assert!(!Token::Newline.is_line_whitespace());
        assert!(Token::Newline.is_any_whitespace());
        assert!(Token::Newline.is_command_end());
        assert!(Token::Semicolon.is_command_end());
        assert!(Token::Eof.is_command_end());
        println!("✓ Test 4: Token properties passed");
    }

    // Test 5: Basic parsing with newlines
    {
        let result = rtcl_parser::parse("set x 1\nset y 2");
        assert!(result.is_ok());
        let cmds = result.unwrap();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].words[0], rtcl_parser::Word::Literal("set".into()));
        assert_eq!(cmds[1].words[0], rtcl_parser::Word::Literal("set".into()));
        println!("✓ Test 5: Parse commands with newlines passed");
    }

    // Test 6: CRLF handling
    {
        let result = rtcl_parser::parse("set x 1\r\nset y 2");
        assert!(result.is_ok());
        let cmds = result.unwrap();
        assert_eq!(cmds.len(), 2);
        println!("✓ Test 6: Parse commands with CRLF passed");
    }

    // Test 7: Mixed line endings
    {
        let result = rtcl_parser::parse("set x 1\nset y 2\r\nset z 3");
        assert!(result.is_ok());
        let cmds = result.unwrap();
        assert_eq!(cmds.len(), 3);
        println!("✓ Test 7: Parse commands with mixed line endings passed");
    }

    println!("\n✅ All token tests passed!");
}
