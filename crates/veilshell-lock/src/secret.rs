//! Locker-private password buffer with volatile zeroization and redacted formatting.

use std::fmt;

use crate::pam::zeroize_bytes;

const MAX_PASSWORD_BYTES: usize = 256;

/// Private password buffer that never prints its contents or length and wipes
/// memory on clear or drop.
#[derive(Default)]
pub struct SecretPassword {
    bytes: Vec<u8>,
}

impl fmt::Debug for SecretPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretPassword([REDACTED])")
    }
}

impl Drop for SecretPassword {
    fn drop(&mut self) {
        self.clear();
    }
}

impl SecretPassword {
    /// Construct from raw UTF-8 text (intended for tests and PAM worker handoff).
    #[cfg(test)]
    #[must_use]
    pub fn from_str_for_test(text: &str) -> Self {
        Self {
            bytes: text.as_bytes().to_vec(),
        }
    }

    /// Append a single printable character if within the buffer limit.
    pub fn push_char(&mut self, ch: char) -> bool {
        if ch.is_control() {
            return false;
        }
        let mut encoded = [0_u8; 4];
        let slice = ch.encode_utf8(&mut encoded).as_bytes();
        if self.bytes.len() + slice.len() > MAX_PASSWORD_BYTES {
            zeroize_bytes(&mut encoded);
            return false;
        }
        self.bytes.extend_from_slice(slice);
        zeroize_bytes(&mut encoded);
        true
    }

    /// Remove the last UTF-8 character and zeroize the vacated bytes.
    pub fn pop_char(&mut self) -> bool {
        let Ok(text) = std::str::from_utf8(&self.bytes) else {
            self.clear();
            return false;
        };
        let Some(last) = text.chars().next_back() else {
            return false;
        };
        let new_len = self.bytes.len() - last.len_utf8();
        zeroize_bytes(&mut self.bytes[new_len..]);
        self.bytes.truncate(new_len);
        true
    }

    /// Wipe all stored bytes in place and empty the buffer.
    pub fn clear(&mut self) {
        zeroize_bytes(&mut self.bytes);
        self.bytes.clear();
    }

    /// Move the current password out for authentication, leaving this buffer empty.
    #[must_use]
    pub fn take_for_auth(&mut self) -> Self {
        Self {
            bytes: std::mem::take(&mut self.bytes),
        }
    }

    /// Raw UTF-8 bytes for PAM conversation submission.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// True if no characters are currently buffered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Number of Unicode scalar values for drawing masked password dots.
    #[must_use]
    pub fn char_count(&self) -> usize {
        std::str::from_utf8(&self.bytes).map_or(0, |s| s.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_never_reveals_secret_or_length() {
        let mut pw = SecretPassword::default();
        pw.push_char('s');
        pw.push_char('3');
        pw.push_char('c');
        let rendered = format!("{pw:?}");
        assert_eq!(rendered, "SecretPassword([REDACTED])");
        assert!(!rendered.contains("s3c"));
    }

    #[test]
    fn push_pop_clear_and_take_behave_cleanly() {
        let mut pw = SecretPassword::default();
        assert!(pw.is_empty());
        assert!(!pw.push_char('\n'));
        assert!(pw.push_char('a'));
        assert!(pw.push_char('é'));
        assert_eq!(pw.char_count(), 2);
        assert!(pw.pop_char());
        assert_eq!(pw.char_count(), 1);
        assert_eq!(pw.as_bytes(), b"a");
        let moved = pw.take_for_auth();
        assert!(pw.is_empty());
        assert_eq!(moved.as_bytes(), b"a");
    }
}
