//! Convert command output to readable log text, rather than interpreting it as a terminal.

/// Preserve line boundaries and diagnostics, removing terminal presentation controls.
/// Also used when displaying older saved results that may still contain escapes.
pub fn readable_output(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\x1b' => match chars.next() {
                Some('[') => {
                    // CSI: parameters/intermediates followed by a final byte.
                    for ch in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&ch) {
                            break;
                        }
                    }
                }
                Some(']' | 'P' | '_' | '^' | 'X') => {
                    // OSC/DCS/APC/PM/SOS strings: never display title/URL/control payloads.
                    while let Some(ch) = chars.next() {
                        if ch == '\x07' || (ch == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                Some(ch) if ('\x20'..='\x2f').contains(&ch) => {
                    for ch in chars.by_ref() {
                        if ('\x30'..='\x7e').contains(&ch) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {
                // Treat redraws as separate log lines; don't overwrite repository errors.
                if chars.peek() != Some(&'\n') {
                    output.push('\n');
                }
            }
            '\n' => output.push('\n'),
            '\t' => output.push_str("    "),
            '\x08' => {
                if !output.ends_with('\n') {
                    output.pop();
                }
            }
            ch if ch.is_control() => {}
            ch => output.push(ch),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redraws_are_lines_and_ansi_and_terminal_payloads_are_removed() {
        let input = "0% [Working]\r\x1b[2K0% [Connecting]\r\x1b[32mHit:1 test-repo\x1b[0m\r\nReading package lists... Done\r\n\x1b]0;window title\x07\x1b]8;;https://example.invalid\x1b\\link\x1b]8;;\x1b\\\n";
        assert_eq!(
            readable_output(input),
            "0% [Working]\n0% [Connecting]\nHit:1 test-repo\nReading package lists... Done\nlink\n"
        );
    }

    #[test]
    fn normalization_preserves_errors_unicode_and_plain_text() {
        let text = "Err:1 repository\r0% [Working]\r\nE: Failed to fetch 套件\n";
        assert_eq!(
            readable_output(text),
            "Err:1 repository\n0% [Working]\nE: Failed to fetch 套件\n"
        );
        assert_eq!(readable_output("abc\x08d\ttext\x00\x07\n"), "abd    text\n");
        assert_eq!(readable_output("Hit:1 repo\nDone\n"), "Hit:1 repo\nDone\n");
        assert_eq!(readable_output("Done\n\x1b["), "Done\n");
    }
}
