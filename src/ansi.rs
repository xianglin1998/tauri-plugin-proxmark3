//! Command output cleanup.
//!
//! The client is a terminal program: it emits ANSI colour sequences, redraws
//! progress lines with `\r` and may print partial lines. What a UI wants is
//! plain text, so everything is normalised here before it reaches JavaScript.

/// Removes ANSI/OSC escape sequences, keeps only the final state of `\r`
/// redrawn lines, drops other control characters and trims trailing
/// whitespace.
pub fn sanitize(input: &str) -> String {
    let stripped = strip_escapes(input);
    let mut output = String::with_capacity(stripped.len());

    for line in stripped.split('\n') {
        // A progress redraw overwrites the same line: keep the last state.
        let visible = match line.rfind('\r') {
            Some(index) => &line[index + 1..],
            None => line,
        };
        for ch in visible.chars() {
            if ch == '\t' || !ch.is_control() {
                output.push(ch);
            }
        }
        output.push('\n');
    }

    output.trim_end_matches('\n').trim_end().to_string()
}

fn strip_escapes(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            output.push(ch);
            continue;
        }

        match chars.next() {
            // CSI: ESC [ parameters final-byte
            Some('[') => {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            // OSC: ESC ] ... BEL | ESC \
            Some(']') => {
                while let Some(next) = chars.next() {
                    if next == '\x07' {
                        break;
                    }
                    if next == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some(_) => {}
            None => break,
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn strips_colour_sequences() {
        assert_eq!(sanitize("\x1b[32m[+]\x1b[0m done"), "[+] done");
    }

    #[test]
    fn keeps_last_progress_state() {
        assert_eq!(sanitize(" 0\r 1\r 2\nnext"), " 2\nnext");
    }

    #[test]
    fn trims_trailing_whitespace() {
        assert_eq!(sanitize("hello\n\n"), "hello");
    }

    #[test]
    fn strips_osc_sequences() {
        assert_eq!(sanitize("\x1b]0;title\x07text"), "text");
    }
}
