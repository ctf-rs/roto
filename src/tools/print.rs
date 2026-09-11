use roto_syntax::{Token, lex};

#[allow(unused)]
mod ansi {
    pub const BLACK: &str = "\x1b[0;30m";
    pub const RED: &str = "\x1b[0;31m";
    pub const GREEN: &str = "\x1b[0;32m";
    pub const YELLOW: &str = "\x1b[0;33m";
    pub const BLUE: &str = "\x1b[0;34m";
    pub const PURPLE: &str = "\x1b[0;35m";
    pub const CYAN: &str = "\x1b[0;36m";
    pub const WHITE: &str = "\x1b[0;37m";
    pub const GRAY: &str = "\x1b[38;5;248m";
    pub const RESET: &str = "\x1b[0m";
    pub const ITALIC: &str = "\x1b[3m";
}

/// Print highlighted Roto code to the terminal
///
/// The syntax highlighting is simple and based solely on the lexer.
pub fn print_highlighted(s: &str) {
    let highlighted = highlight(s);
    print!("{highlighted}");
}

fn highlight(s: &str) -> String {
    let mut highlighted = String::with_capacity(s.len());
    let mut last_end = 0;
    for token in lex(s) {
        let range = token.span;
        if range.start > last_end {
            highlighted.push_str(ansi::GRAY);
            highlighted.push_str(ansi::ITALIC);
            highlighted.push_str(&s[last_end..range.start]);
            highlighted.push_str(ansi::RESET);
        }
        last_end = range.end;
        let color = match token.token {
            Some(Token::Ident(_)) => ansi::WHITE,
            Some(
                Token::AmpAmp
                | Token::AngleLeftEq
                | Token::AngleRightEq
                | Token::Arrow
                | Token::Bang
                | Token::BangEq
                | Token::Colon
                | Token::Comma
                | Token::Eq
                | Token::EqEq
                | Token::FatArrow
                | Token::Hash
                | Token::Hyphen
                | Token::HyphenHyphen
                | Token::Period
                | Token::Pipe
                | Token::PipePipe
                | Token::Plus
                | Token::QuestionMark
                | Token::SemiColon
                | Token::Slash
                | Token::BlockComment(_)
                | Token::Star
                | Token::AngleLeft
                | Token::AngleRight
                | Token::CurlyLeft
                | Token::CurlyRight
                | Token::RoundLeft
                | Token::RoundRight
                | Token::SquareLeft
                | Token::SquareRight
                | Token::Percent
                | Token::PlusEq
                | Token::MinusEq
                | Token::StarEq
                | Token::SlashEq
                | Token::PercentEq,
            ) => ansi::GRAY,
            Some(Token::Keyword(_)) => ansi::BLUE,
            Some(
                Token::String(_)
                | Token::Char(_)
                | Token::FStringStart
                | Token::FStringText(_)
                | Token::FStringEnd(_),
            ) => ansi::GREEN,
            Some(
                Token::Integer(_, _)
                | Token::Float(_, _)
                | Token::Hex(_)
                | Token::Asn(_)
                | Token::IpV4(_)
                | Token::IpV6(_)
                | Token::Bool(_),
            ) => ansi::PURPLE,
            None => ansi::RED,
        };
        highlighted.push_str(color);
        highlighted.push_str(&s[range]);
        highlighted.push_str(ansi::RESET);
    }

    if last_end < s.len() {
        highlighted.push_str(ansi::GRAY);
        highlighted.push_str(ansi::ITALIC);
        highlighted.push_str(&s[last_end..]);
        highlighted.push_str(ansi::RESET);
    }

    highlighted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn without_ansi(mut text: String) -> String {
        for code in [
            ansi::RED,
            ansi::GREEN,
            ansi::BLUE,
            ansi::PURPLE,
            ansi::WHITE,
            ansi::GRAY,
            ansi::ITALIC,
            ansi::RESET,
        ] {
            text = text.replace(code, "");
        }
        text
    }

    #[test]
    fn highlights_complete_f_string_token_stream() {
        let source = "print(f\"Hello {name}!\");\n";
        let highlighted = highlight(source);

        assert_eq!(without_ansi(highlighted.clone()), source);
        assert!(highlighted.contains(&format!(
            "{}Hello {}",
            ansi::GREEN,
            ansi::RESET
        )));
        assert!(highlighted.contains(&format!(
            "{}!\"{}",
            ansi::GREEN,
            ansi::RESET
        )));
    }

    #[test]
    fn invalid_tokens_are_highlighted_without_panicking() {
        let source = "☃ ok\n";
        let highlighted = highlight(source);

        assert_eq!(without_ansi(highlighted.clone()), source);
        assert!(highlighted.contains(&format!(
            "{}☃{}",
            ansi::RED,
            ansi::RESET
        )));
    }
}
